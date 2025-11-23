use anyhow::Result;
use rememberme_core::Chunk;
use rememberme_llm::MiniLLM;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Context analyzer that selects the most relevant chunks using LLM
pub struct ContextAnalyzer {
    llm: Arc<MiniLLM>,
}

impl ContextAnalyzer {
    /// Create a new context analyzer
    pub fn new(llm: Arc<MiniLLM>) -> Self {
        Self { llm }
    }
    
    /// Select the most relevant chunks from RAG results
    pub async fn select_relevant(
        &mut self,
        query: &str,
        chunks: Vec<Chunk>,
        top_k: usize,
    ) -> Result<Vec<Chunk>> {
        if chunks.is_empty() {
            return Ok(Vec::new());
        }
        
        if chunks.len() <= top_k {
            return Ok(chunks);
        }
        
        // Format chunks for LLM
        let chunks_text = chunks
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let content = if c.content.len() > 200 {
                    format!("{}...", &c.content[..200])
                } else {
                    c.content.clone()
                };
                format!("{}. [{}] {}", i + 1, c.document_id, content)
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        
        let system_prompt = "You are a context relevance analyzer. \
                            Given a query and multiple code/document chunks, \
                            identify which chunks are most relevant. \
                            Respond ONLY with valid JSON.";
        
        let user_prompt = format!(
            r#"Query: "{}"

Available chunks:
{}

Select the top {} most relevant chunks for answering this query.
Respond with ONLY a JSON array of chunk numbers (1-indexed), like: [1, 3, 5]

Consider:
- Direct relevance to the query
- Contains code/info needed to answer
- Avoid redundant chunks

Response (JSON array only):"#,
            query,
            chunks_text,
            top_k.min(chunks.len())
        );
        
        // Get LLM response
        let llm = Arc::get_mut(&mut self.llm)
            .ok_or_else(|| anyhow::anyhow!("Failed to get mutable LLM reference"))?;
        
        let response = llm
            .generate_with_system(&system_prompt, &user_prompt, 100)
            .await?;
        
        // Parse indices
        let indices = self.parse_indices(&response, chunks.len())?;
        
        // Select chunks
        let selected: Vec<Chunk> = indices
            .iter()
            .take(top_k)
            .filter_map(|&idx| chunks.get(idx).cloned())
            .collect();
        
        Ok(selected)
    }
    
    /// Parse chunk indices from LLM response
    fn parse_indices(&self, response: &str, max_index: usize) -> Result<Vec<usize>> {
        // Find JSON array in response
        let start = response.find('[').unwrap_or(0);
        let end = response.rfind(']').map(|i| i + 1).unwrap_or(response.len());
        let json_str = &response[start..end];
        
        // Parse JSON
        let parsed: Vec<usize> = serde_json::from_str(json_str)
            .or_else(|_| {
                // Fallback: try extracting numbers manually
                let nums: Vec<usize> = response
                    .chars()
                    .filter(|c| c.is_numeric() || *c == ',')
                    .collect::<String>()
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                Ok::<Vec<usize>, anyhow::Error>(nums)
            })?;
        
        // Convert to 0-indexed and validate
        let indices: Vec<usize> = parsed
            .into_iter()
            .filter_map(|i| {
                if i > 0 && i <= max_index {
                    Some(i - 1) // Convert to 0-indexed
                } else {
                    None
                }
            })
            .collect();
        
        if indices.is_empty() {
            // Fallback: return first chunks
            Ok((0..max_index.min(3)).collect())
        } else {
            Ok(indices)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_indices() {
        let llm = Arc::new(MiniLLM::new(Default::default()).unwrap());
        let analyzer = ContextAnalyzer::new(llm);
        
        // Valid JSON array
        let indices = analyzer.parse_indices("[1, 3, 5]", 10).unwrap();
        assert_eq!(indices, vec![0, 2, 4]); // 0-indexed
        
        // With extra text
        let indices = analyzer.parse_indices("The most relevant are [2, 4]", 10).unwrap();
        assert_eq!(indices, vec![1, 3]);
        
        // Fallback for invalid
        let indices = analyzer.parse_indices("invalid", 5).unwrap();
        assert!(!indices.is_empty()); // Should return fallback
    }
}
