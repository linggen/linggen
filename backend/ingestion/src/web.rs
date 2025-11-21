use anyhow::Result;
use async_trait::async_trait;
use rememberme_core::{Document, SourceType};
use reqwest::Client;
use scraper::{Html, Selector};
use std::collections::HashSet;
use url::Url;
use uuid::Uuid;

use crate::Ingestor;

pub struct WebIngestor {
    pub base_url: String,
    pub max_depth: usize,
}

impl WebIngestor {
    pub fn new(base_url: String, max_depth: usize) -> Self {
        Self {
            base_url,
            max_depth,
        }
    }

    async fn crawl(
        &self,
        client: &Client,
        url: Url,
        depth: usize,
        visited: &mut HashSet<String>,
    ) -> Result<Vec<Document>> {
        if depth > self.max_depth || visited.contains(url.as_str()) {
            return Ok(Vec::new());
        }

        visited.insert(url.to_string());
        let mut documents = Vec::new();

        // Fetch page
        let resp = client.get(url.clone()).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        let html_content = resp.text().await?;

        // Scope for Html parsing to ensure it's dropped before await
        let (text_content, links) = {
            let document = Html::parse_document(&html_content);

            // Extract text
            let body_selector = Selector::parse("body").unwrap();
            let text = if let Some(body) = document.select(&body_selector).next() {
                body.text().collect::<Vec<_>>().join(" ")
            } else {
                String::new()
            };

            // Find links
            let a_selector = Selector::parse("a").unwrap();
            let mut found_links = Vec::new();

            if depth < self.max_depth {
                for element in document.select(&a_selector) {
                    if let Some(href) = element.value().attr("href") {
                        if let Ok(next_url) = url.join(href) {
                            if next_url.domain() == url.domain() {
                                found_links.push(next_url);
                            }
                        }
                    }
                }
            }
            (text, found_links)
        };

        let doc = Document {
            id: Uuid::new_v4().to_string(),
            source_type: SourceType::Web,
            source_url: url.to_string(),
            content: text_content,
            metadata: serde_json::json!({
                "url": url.to_string(),
                "depth": depth,
            }),
        };
        documents.push(doc);

        for link in links {
            // Box::pin for recursion
            let mut child_docs = Box::pin(self.crawl(client, link, depth + 1, visited)).await?;
            documents.append(&mut child_docs);
        }

        Ok(documents)
    }
}

#[async_trait]
impl Ingestor for WebIngestor {
    async fn ingest(&self) -> Result<Vec<Document>> {
        let client = Client::new();
        let start_url = Url::parse(&self.base_url)?;
        let mut visited = HashSet::new();

        self.crawl(&client, start_url, 0, &mut visited).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_web_ingestion() -> Result<()> {
        let mock_server = MockServer::start().await;

        // Mock index page
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"
                <html>
                    <body>
                        <h1>Welcome</h1>
                        <p>This is the index page.</p>
                        <a href="/page1">Go to Page 1</a>
                    </body>
                </html>
            "#,
            ))
            .mount(&mock_server)
            .await;

        // Mock page 1
        Mock::given(method("GET"))
            .and(path("/page1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"
                <html>
                    <body>
                        <h1>Page 1</h1>
                        <p>This is page 1.</p>
                    </body>
                </html>
            "#,
            ))
            .mount(&mock_server)
            .await;

        let ingestor = WebIngestor::new(mock_server.uri(), 1);
        let docs = ingestor.ingest().await?;

        assert_eq!(docs.len(), 2);

        // Verify index page
        let index_doc = docs
            .iter()
            .find(|d| d.source_url == mock_server.uri() + "/")
            .unwrap();
        assert!(index_doc.content.contains("Welcome"));

        // Verify page 1
        let page1_doc = docs
            .iter()
            .find(|d| d.source_url == mock_server.uri() + "/page1")
            .unwrap();
        assert!(page1_doc.content.contains("Page 1"));

        Ok(())
    }
}
