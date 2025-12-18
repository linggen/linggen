use anyhow::Result;
use embeddings::{EmbeddingModel, TextChunker};
use linggen_core::Chunk;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

/// Extract title from markdown frontmatter (YAML between --- delimiters)
fn extract_title_from_content(content: &str) -> Option<String> {
    if !content.starts_with("---") {
        return None;
    }

    let mut parts = content.splitn(3, "---");
    parts.next(); // skip empty before first ---
    let frontmatter = parts.next()?;

    // Simple YAML parsing for title field
    for line in frontmatter.lines() {
        let line = line.trim();
        if line.starts_with("title:") {
            let title = line.strip_prefix("title:")?.trim();
            // Remove quotes if present
            let title = title.trim_matches(|c| c == '"' || c == '\'');
            return Some(title.to_string());
        }
    }

    None
}

/// Index a memory or prompt file into the internal index
/// This is called after saving/updating a memory/prompt file
pub async fn index_internal_file(
    internal_index_store: &storage::InternalIndexStore,
    embedding_model: &Arc<RwLock<Option<EmbeddingModel>>>,
    chunker: &TextChunker,
    source_id: &str,
    file_path: &Path,
    relative_path: &str,
) -> Result<()> {
    // Read file content
    let content = match tokio::fs::read_to_string(file_path).await {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read internal file {:?}: {}", file_path, e);
            return Err(e.into());
        }
    };

    // Determine kind based on path
    let kind = if relative_path.contains("/memory/") || relative_path.starts_with("memory/") {
        "memory"
    } else if relative_path.contains("/prompts/") || relative_path.starts_with("prompts/") {
        "prompt"
    } else if relative_path.contains("/notes/") || relative_path.starts_with("notes/") {
        "note"
    } else {
        "other"
    };

    // Create unique document_id: {source_id}/{kind}/{path}
    let document_id = format!("{}/{}/{}", source_id, kind, relative_path);

    // Extract title from frontmatter for memories
    let title = extract_title_from_content(&content);

    // Chunk the content
    let text_chunks = chunker.chunk(&content);

    if text_chunks.is_empty() {
        // Empty file, just delete any existing chunks
        internal_index_store.delete_document(&document_id).await?;
        return Ok(());
    }

    // Create chunks with embeddings
    let mut chunks = Vec::new();

    // Get embedding model
    let model_guard = embedding_model.read().await;
    if let Some(model) = model_guard.as_ref() {
        // Get embeddings for all chunks in batch
        // Convert Vec<String> to Vec<&str> for embed_batch
        let text_refs: Vec<&str> = text_chunks.iter().map(|s| s.as_str()).collect();
        let embedding_result = model.embed_batch(&text_refs);

        match embedding_result {
            Ok(embeddings) => {
                for (i, text_chunk) in text_chunks.iter().enumerate() {
                    let embedding = embeddings.get(i).cloned();

                    let mut metadata = serde_json::json!({
                        "kind": kind,
                        "file_path": relative_path,
                    });
                    if let Some(ref t) = title {
                        metadata["title"] = serde_json::Value::String(t.clone());
                    }

                    chunks.push(Chunk {
                        id: Uuid::new_v4(),
                        source_id: source_id.to_string(),
                        document_id: document_id.clone(),
                        content: text_chunk.clone(),
                        embedding,
                        metadata,
                    });
                }
            }
            Err(e) => {
                warn!("Failed to generate embeddings for internal file: {}", e);
                // Create chunks without embeddings
                for text_chunk in text_chunks.iter() {
                    let mut metadata = serde_json::json!({
                        "kind": kind,
                        "file_path": relative_path,
                    });
                    if let Some(ref t) = title {
                        metadata["title"] = serde_json::Value::String(t.clone());
                    }

                    chunks.push(Chunk {
                        id: Uuid::new_v4(),
                        source_id: source_id.to_string(),
                        document_id: document_id.clone(),
                        content: text_chunk.clone(),
                        embedding: None,
                        metadata,
                    });
                }
            }
        }
    } else {
        // No embedding model, create chunks without embeddings
        for text_chunk in text_chunks.iter() {
            let mut metadata = serde_json::json!({
                "kind": kind,
                "file_path": relative_path,
            });
            if let Some(ref t) = title {
                metadata["title"] = serde_json::Value::String(t.clone());
            }

            chunks.push(Chunk {
                id: Uuid::new_v4(),
                source_id: source_id.to_string(),
                document_id: document_id.clone(),
                content: text_chunk.clone(),
                embedding: None,
                metadata,
            });
        }
    }

    // Upsert into internal index (this deletes old chunks and adds new ones)
    internal_index_store
        .upsert_document(&document_id, chunks)
        .await?;

    info!(
        "Indexed internal file {} ({} chunks)",
        relative_path,
        text_chunks.len()
    );

    Ok(())
}

/// Remove an internal file from the index
pub async fn remove_internal_file(
    internal_index_store: &storage::InternalIndexStore,
    source_id: &str,
    kind: &str,
    relative_path: &str,
) -> Result<()> {
    let document_id = format!("{}/{}/{}", source_id, kind, relative_path);
    internal_index_store.delete_document(&document_id).await?;
    info!("Removed internal file from index: {}", document_id);
    Ok(())
}

/// Rescan and reindex all memory/prompt files for a source
/// This is useful for out-of-band edits or initial population
pub async fn rescan_internal_files(
    internal_index_store: &storage::InternalIndexStore,
    embedding_model: &Arc<RwLock<Option<EmbeddingModel>>>,
    chunker: &TextChunker,
    source_id: &str,
    source_path: &str,
) -> Result<(usize, usize)> {
    use std::path::PathBuf;

    let linggen_dir = PathBuf::from(source_path).join(".linggen");
    let memory_dir = linggen_dir.join("memory");
    let prompts_dir = linggen_dir.join("prompts");
    let notes_dir = linggen_dir.join("notes");

    let mut files_indexed = 0;
    let mut files_failed = 0;

    // Index memory files
    if memory_dir.exists() {
        info!("Rescanning memory files in {:?}", memory_dir);
        let result = rescan_directory(
            internal_index_store,
            embedding_model,
            chunker,
            source_id,
            &memory_dir,
            &linggen_dir,
        )
        .await;

        match result {
            Ok((indexed, failed)) => {
                files_indexed += indexed;
                files_failed += failed;
            }
            Err(e) => {
                warn!("Failed to rescan memory directory: {}", e);
            }
        }
    }

    // Index prompt files
    if prompts_dir.exists() {
        info!("Rescanning prompt files in {:?}", prompts_dir);
        let result = rescan_directory(
            internal_index_store,
            embedding_model,
            chunker,
            source_id,
            &prompts_dir,
            &linggen_dir,
        )
        .await;

        match result {
            Ok((indexed, failed)) => {
                files_indexed += indexed;
                files_failed += failed;
            }
            Err(e) => {
                warn!("Failed to rescan prompts directory: {}", e);
            }
        }
    }

    // Index notes files
    if notes_dir.exists() {
        info!("Rescanning notes files in {:?}", notes_dir);
        let result = rescan_directory(
            internal_index_store,
            embedding_model,
            chunker,
            source_id,
            &notes_dir,
            &linggen_dir,
        )
        .await;

        match result {
            Ok((indexed, failed)) => {
                files_indexed += indexed;
                files_failed += failed;
            }
            Err(e) => {
                warn!("Failed to rescan notes directory: {}", e);
            }
        }
    }

    Ok((files_indexed, files_failed))
}

fn rescan_directory<'a>(
    internal_index_store: &'a storage::InternalIndexStore,
    embedding_model: &'a Arc<RwLock<Option<EmbeddingModel>>>,
    chunker: &'a TextChunker,
    source_id: &'a str,
    dir: &'a Path,
    base_dir: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(usize, usize)>> + Send + 'a>> {
    Box::pin(async move {
        let mut files_indexed = 0;
        let mut files_failed = 0;

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read directory {:?}: {}", dir, e);
                return Ok((0, 0));
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();

            if path.is_dir() {
                // Recursive
                let (indexed, failed) = rescan_directory(
                    internal_index_store,
                    embedding_model,
                    chunker,
                    source_id,
                    &path,
                    base_dir,
                )
                .await?;
                files_indexed += indexed;
                files_failed += failed;
            } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                // Index markdown file
                let relative_path = path
                    .strip_prefix(base_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();

                match index_internal_file(
                    internal_index_store,
                    embedding_model,
                    chunker,
                    source_id,
                    &path,
                    &relative_path,
                )
                .await
                {
                    Ok(_) => files_indexed += 1,
                    Err(e) => {
                        warn!("Failed to index internal file {}: {}", relative_path, e);
                        files_failed += 1;
                    }
                }
            }
        }

        Ok((files_indexed, files_failed))
    })
}
