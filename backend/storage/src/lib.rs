use anyhow::Result;
use arrow_array::types::Float32Type;
use arrow_array::{FixedSizeListArray, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{connect, Connection};
use rememberme_core::Chunk;
use std::sync::Arc;

pub mod metadata;
pub use metadata::MetadataStore;

pub struct VectorStore {
    conn: Connection,
    table_name: String,
}

impl VectorStore {
    pub async fn new(uri: &str) -> Result<Self> {
        let conn = connect(uri).execute().await?;
        Ok(Self {
            conn,
            table_name: "chunks".to_string(),
        })
    }

    pub async fn init(&self) -> Result<()> {
        // Define schema
        // id: Utf8
        // content: Utf8
        // vector: FixedSizeList(Float32, 384) - assuming 384 dim for now (e.g. all-MiniLM-L6-v2)
        // We might need to make dimension configurable

        // For now, we'll just check if table exists. Creating it requires data or explicit schema.
        // LanceDB creates tables lazily or with initial data usually.
        Ok(())
    }

    pub async fn add(&self, chunks: Vec<Chunk>) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        // Assuming all chunks have embeddings of same size
        let dim = chunks[0].embedding.as_ref().map(|v| v.len()).unwrap_or(384);

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    dim as i32,
                ),
                true,
            ),
        ]));

        let ids: Vec<String> = chunks.iter().map(|c| c.id.to_string()).collect();
        let contents: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();

        let id_array = StringArray::from(ids);
        let content_array = StringArray::from(contents);

        // Create vector array
        // FixedSizeListArray::from_iter_primitive expects an iterator of Option<P>
        let vector_array = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
            chunks.iter().map(|c| {
                if let Some(emb) = &c.embedding {
                    Some(emb.iter().map(|&x| Some(x)).collect::<Vec<_>>())
                } else {
                    Some(vec![None; dim])
                }
            }),
            dim as i32,
        );

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(id_array),
                Arc::new(content_array),
                Arc::new(vector_array),
            ],
        )?;

        // lancedb 0.22 uses RecordBatchIterator
        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema.clone());

        // Create or append
        // Check if table exists
        let table_names = self.conn.table_names().execute().await?;
        if table_names.contains(&self.table_name) {
            let table = self.conn.open_table(&self.table_name).execute().await?;
            table.add(batches).execute().await?;
        } else {
            self.conn
                .create_table(&self.table_name, batches)
                .execute()
                .await?;
        }

        Ok(())
    }

    pub async fn create_fts_index(&self) -> Result<()> {
        let _table = self.conn.open_table(&self.table_name).execute().await?;
        // Check if FTS index exists or create it.
        // Note: lancedb 0.4 might not expose FTS creation easily in Rust yet.
        // We'll try to use the generic create_index if possible, or just skip for now if not supported.
        // For this iteration, I'll assume we might need to wait for better FTS support or use tantivy directly.
        // Let's try to see if we can just compile a reference to IndexType::FTS.
        Ok(())
    }

    pub async fn search(
        &self,
        query_embedding: Vec<f32>,
        query_text: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Chunk>> {
        let table = self.conn.open_table(&self.table_name).execute().await?;

        let mut query = table.vector_search(query_embedding)?;
        query = query.limit(limit);

        // If we had FTS, we would do something like:
        // if let Some(text) = query_text {
        //     query = query.text(text); // hypothetical API
        // }

        let results = query.execute().await?;

        let mut chunks = Vec::new();
        let mut stream = results;

        while let Some(batch) = stream.try_next().await? {
            let id_col = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let content_col = batch
                .column_by_name("content")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();

            for i in 0..batch.num_rows() {
                chunks.push(Chunk {
                    id: uuid::Uuid::parse_str(id_col.value(i)).unwrap_or_default(),
                    document_id: "".to_string(), // TODO
                    content: content_col.value(i).to_string(),
                    embedding: None,
                    metadata: serde_json::Value::Null,
                });
            }
        }

        // Poor man's re-ranking if we have query text but no FTS index yet:
        if let Some(text) = query_text {
            let text = text.to_lowercase();
            // Simple keyword boost: move exact matches to top
            chunks.sort_by(|a, b| {
                let a_contains = a.content.to_lowercase().contains(&text);
                let b_contains = b.content.to_lowercase().contains(&text);
                b_contains.cmp(&a_contains)
            });
        }

        Ok(chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_lancedb_integration() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let uri = temp_dir.path().to_str().unwrap();

        let store = VectorStore::new(uri).await?;

        // Create dummy chunk
        let chunk = Chunk {
            id: uuid::Uuid::new_v4(),
            document_id: "doc1".to_string(),
            content: "Hello LanceDB".to_string(),
            embedding: Some(vec![0.1; 384]),
            metadata: serde_json::Value::Null,
        };

        store.add(vec![chunk]).await?;

        // Search
        let results = store.search(vec![0.1; 384], Some("LanceDB"), 1).await?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "Hello LanceDB");

        Ok(())
    }

    #[tokio::test]
    async fn test_hybrid_search_ranking() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let uri = temp_dir.path().to_str().unwrap();
        let store = VectorStore::new(uri).await?;

        let chunk1 = Chunk {
            id: uuid::Uuid::new_v4(),
            document_id: "doc1".to_string(),
            content: "Apple banana cherry".to_string(),
            embedding: Some(vec![0.1; 384]),
            metadata: serde_json::Value::Null,
        };
        let chunk2 = Chunk {
            id: uuid::Uuid::new_v4(),
            document_id: "doc2".to_string(),
            content: "Banana date elderberry".to_string(),
            embedding: Some(vec![0.1; 384]),
            metadata: serde_json::Value::Null,
        };

        store.add(vec![chunk1, chunk2]).await?;

        // Search for "apple" - chunk1 should be first
        let results = store.search(vec![0.1; 384], Some("apple"), 2).await?;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].content, "Apple banana cherry");

        // Search for "date" - chunk2 should be first
        let results = store.search(vec![0.1; 384], Some("date"), 2).await?;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].content, "Banana date elderberry");

        Ok(())
    }
}
