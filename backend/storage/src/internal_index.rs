use anyhow::Result;
use arrow_array::types::Float32Type;
use arrow_array::{Array, FixedSizeListArray, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::Connection;
use linggen_core::Chunk;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Internal index for Linggen's own content (memories, prompts, etc.)
/// Uses a separate LanceDB table to avoid mixing with project/source code index
pub struct InternalIndexStore {
    conn: Connection,
    table_name: String,
}

impl InternalIndexStore {
    pub async fn new(uri: &str) -> Result<Self> {
        // Create directory if it doesn't exist (for file-based URIs)
        if !uri.starts_with("http") && !uri.starts_with("s3") {
            std::fs::create_dir_all(uri)?;
        }

        let conn = lancedb::connect(uri).execute().await?;
        Ok(Self {
            conn,
            table_name: "internal_chunks".to_string(),
        })
    }

    /// Upsert a memory or prompt file: delete old chunks for this document, then add new ones
    /// This ensures we stay in sync with the file content
    pub async fn upsert_document(&self, document_id: &str, chunks: Vec<Chunk>) -> Result<()> {
        if chunks.is_empty() {
            // If no chunks provided, this is effectively a delete
            return self.delete_document(document_id).await;
        }

        info!(
            "Upserting {} chunks for internal document: {}",
            chunks.len(),
            document_id
        );

        // 1. Delete existing chunks for this document
        self.delete_document(document_id).await?;

        // 2. Add new chunks
        self.add_chunks(chunks).await?;

        Ok(())
    }

    /// Delete all chunks for a specific internal document (memory/prompt file)
    pub async fn delete_document(&self, document_id: &str) -> Result<()> {
        let table_names = self.conn.table_names().execute().await?;
        if !table_names.contains(&self.table_name) {
            // Table doesn't exist yet, nothing to delete
            return Ok(());
        }

        let table = self.conn.open_table(&self.table_name).execute().await?;
        let filter = format!("document_id = '{}'", document_id);

        match table.delete(&filter).await {
            Ok(_) => {
                debug!("Deleted internal chunks for document: {}", document_id);
                Ok(())
            }
            Err(e) => {
                warn!(
                    "Failed to delete internal chunks for {}: {}",
                    document_id, e
                );
                Err(e.into())
            }
        }
    }

    /// Add chunks to the internal index
    async fn add_chunks(&self, chunks: Vec<Chunk>) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        let dim = chunks[0].embedding.as_ref().map(|v| v.len()).unwrap_or(384);

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("source_id", DataType::Utf8, false),
            Field::new("document_id", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("kind", DataType::Utf8, true), // "memory" or "prompt"
            Field::new("file_path", DataType::Utf8, true), // relative path under .linggen/
            Field::new("title", DataType::Utf8, true), // extracted title
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
        let source_ids: Vec<String> = chunks.iter().map(|c| c.source_id.clone()).collect();
        let document_ids: Vec<String> = chunks.iter().map(|c| c.document_id.clone()).collect();
        let contents: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();

        // Extract kind, file_path, title from metadata
        let kinds: Vec<Option<String>> = chunks
            .iter()
            .map(|c| {
                c.metadata
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        let file_paths: Vec<Option<String>> = chunks
            .iter()
            .map(|c| {
                c.metadata
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        let titles: Vec<Option<String>> = chunks
            .iter()
            .map(|c| {
                c.metadata
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect();

        let id_array = StringArray::from(ids);
        let source_id_array = StringArray::from(source_ids);
        let document_id_array = StringArray::from(document_ids);
        let content_array = StringArray::from(contents);
        let kind_array = StringArray::from(kinds);
        let file_path_array = StringArray::from(file_paths);
        let title_array = StringArray::from(titles);

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
                Arc::new(source_id_array),
                Arc::new(document_id_array),
                Arc::new(content_array),
                Arc::new(kind_array),
                Arc::new(file_path_array),
                Arc::new(title_array),
                Arc::new(vector_array),
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema.clone());

        // Create or append to internal_chunks table
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

    /// Search the internal index
    pub async fn search(
        &self,
        query_embedding: Vec<f32>,
        query_text: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Chunk>> {
        info!(
            "🔎 [InternalIndex] Search called - embedding dim: {}, query_text: {:?}, limit: {}",
            query_embedding.len(),
            query_text,
            limit
        );

        let table_names = self.conn.table_names().execute().await?;
        info!("📋 [InternalIndex] Available tables: {:?}", table_names);

        if !table_names.contains(&self.table_name) {
            warn!(
                "⚠️  [InternalIndex] Table '{}' doesn't exist yet, returning empty results",
                self.table_name
            );
            return Ok(Vec::new());
        }

        info!("✅ [InternalIndex] Opening table '{}'...", self.table_name);
        let table = self.conn.open_table(&self.table_name).execute().await?;

        info!("🔍 [InternalIndex] Executing vector search...");
        let mut query = table.vector_search(query_embedding)?;
        query = query.limit(limit);

        let results = query.execute().await?;
        info!("✅ [InternalIndex] Vector search executed");

        let mut chunks = Vec::new();
        let mut stream = results;
        let mut batch_count = 0;

        while let Some(batch) = stream.try_next().await? {
            batch_count += 1;
            info!(
                "📦 [InternalIndex] Processing batch {} with {} rows",
                batch_count,
                batch.num_rows()
            );
            let id_col = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let source_id_col = batch
                .column_by_name("source_id")
                .map(|col| col.as_any().downcast_ref::<StringArray>().unwrap());
            let document_id_col = batch
                .column_by_name("document_id")
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
            let kind_col = batch
                .column_by_name("kind")
                .map(|col| col.as_any().downcast_ref::<StringArray>().unwrap());
            let file_path_col = batch
                .column_by_name("file_path")
                .map(|col| col.as_any().downcast_ref::<StringArray>().unwrap());
            let title_col = batch
                .column_by_name("title")
                .map(|col| col.as_any().downcast_ref::<StringArray>().unwrap());

            for i in 0..batch.num_rows() {
                let mut metadata = serde_json::Map::new();

                if let Some(col) = kind_col {
                    if !col.is_null(i) {
                        metadata.insert(
                            "kind".to_string(),
                            serde_json::Value::String(col.value(i).to_string()),
                        );
                    }
                }
                if let Some(col) = file_path_col {
                    if !col.is_null(i) {
                        metadata.insert(
                            "file_path".to_string(),
                            serde_json::Value::String(col.value(i).to_string()),
                        );
                    }
                }
                if let Some(col) = title_col {
                    if !col.is_null(i) {
                        metadata.insert(
                            "title".to_string(),
                            serde_json::Value::String(col.value(i).to_string()),
                        );
                    }
                }

                chunks.push(Chunk {
                    id: Uuid::parse_str(id_col.value(i)).unwrap_or_default(),
                    source_id: source_id_col
                        .map(|c| c.value(i).to_string())
                        .unwrap_or_default(),
                    document_id: document_id_col.value(i).to_string(),
                    content: content_col.value(i).to_string(),
                    embedding: None,
                    metadata: serde_json::Value::Object(metadata),
                });
            }
        }

        info!("📊 [InternalIndex] Collected {} chunks total", chunks.len());

        // Simple keyword boost if query text provided
        if let Some(text) = query_text {
            let text = text.to_lowercase();
            chunks.sort_by(|a, b| {
                let a_contains = a.content.to_lowercase().contains(&text);
                let b_contains = b.content.to_lowercase().contains(&text);
                b_contains.cmp(&a_contains)
            });
            info!("🔄 [InternalIndex] Applied keyword boost for query text");
        }

        info!(
            "✅ [InternalIndex] Search complete, returning {} chunks",
            chunks.len()
        );
        Ok(chunks)
    }

    /// List all documents in the internal index
    pub async fn list_documents(&self) -> Result<Vec<String>> {
        let table_names = self.conn.table_names().execute().await?;
        if !table_names.contains(&self.table_name) {
            return Ok(Vec::new());
        }

        let table = self.conn.open_table(&self.table_name).execute().await?;
        let results = table.query().execute().await?;

        let mut document_ids = std::collections::HashSet::new();
        let mut stream = results;

        while let Some(batch) = stream.try_next().await? {
            let document_id_col = batch
                .column_by_name("document_id")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();

            for i in 0..batch.num_rows() {
                document_ids.insert(document_id_col.value(i).to_string());
            }
        }

        let mut docs: Vec<String> = document_ids.into_iter().collect();
        docs.sort();
        Ok(docs)
    }

    /// Clear all internal chunks (for maintenance/testing)
    pub async fn clear_all(&self) -> Result<()> {
        let table_names = self.conn.table_names().execute().await?;
        if table_names.contains(&self.table_name) {
            self.conn.drop_table(&self.table_name, &[]).await?;
            info!("Dropped internal_chunks table");
        }
        Ok(())
    }
}
