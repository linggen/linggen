use anyhow::Result;
use embeddings::{EmbeddingModel, TextChunker};
use linggen_core::Chunk;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

/// Start a background file watcher for a project's .linggen directory
pub async fn start_internal_watcher(
    internal_index_store: Arc<storage::InternalIndexStore>,
    embedding_model: Arc<RwLock<Option<EmbeddingModel>>>,
    chunker: Arc<TextChunker>,
    broadcast_tx: tokio::sync::broadcast::Sender<serde_json::Value>,
    source_id: String,
    linggen_dir: PathBuf,
) -> Result<()> {
    use ingestion::FileWatcher;

    let mut watcher = FileWatcher::new(&linggen_dir)?;
    info!(
        "Started internal watcher for source {} at {:?}",
        source_id, linggen_dir
    );

    let internal_index_store_broadcast = broadcast_tx.clone();

    tokio::spawn(async move {
        while let Some(res) = watcher.next_event().await {
            match res {
                Ok(event) => {
                    for path in event.paths {
                        // Skip directories
                        if path.is_dir() {
                            continue;
                        }

                        // We only care about markdown files in specific subdirs
                        if path.extension().map(|e| e == "md").unwrap_or(false) {
                            let rel_str = if let Ok(rel) = path.strip_prefix(&linggen_dir) {
                                Some(rel.to_string_lossy().to_string())
                            } else {
                                // Fallback for edge cases where strip_prefix fails
                                let path_str = path.to_string_lossy();
                                let root_str = linggen_dir.to_string_lossy();
                                if path_str.starts_with(&*root_str) {
                                    Some(path_str[root_str.len()..].to_string())
                                } else {
                                    None
                                }
                            };

                            if let Some(mut rel_str) = rel_str {
                                // Ensure no leading slash
                                if rel_str.starts_with('/') {
                                    rel_str = rel_str[1..].to_string();
                                }

                                // Check if it's in one of our tracked folders
                                if !rel_str.starts_with("memory/")
                                    && !rel_str.starts_with("prompts/")
                                    && !rel_str.starts_with("notes/")
                                {
                                    continue;
                                }

                                // Determine if we should treat this as a removal or an update
                                // On some OSes (like macOS), renames/deletes can show up as Modify/Name
                                // If the file no longer exists, it's a removal.
                                let is_removal = event.kind.is_remove() || !path.exists();

                                if is_removal {
                                    let kind = if rel_str.starts_with("memory/") {
                                        "memory"
                                    } else if rel_str.starts_with("prompts/") {
                                        "prompt"
                                    } else {
                                        "note"
                                    };

                                    if let Err(e) = remove_internal_file(
                                        &internal_index_store,
                                        &source_id,
                                        kind,
                                        &rel_str,
                                    )
                                    .await
                                    {
                                        warn!("Watcher: failed to remove {}: {}", rel_str, e);
                                    }

                                    // Always notify UI of removal
                                    let msg = serde_json::json!({
                                        "event": "file_removed",
                                        "path": rel_str,
                                        "source_id": source_id,
                                    });
                                    let _ = internal_index_store_broadcast.send(msg);
                                } else {
                                    // Index the file
                                    match index_internal_file(
                                        &internal_index_store,
                                        &embedding_model,
                                        &chunker,
                                        &source_id,
                                        &path,
                                        &rel_str,
                                    )
                                    .await
                                    {
                                        Ok(_) => {
                                            let msg = serde_json::json!({
                                                "event": "file_changed",
                                                "path": rel_str,
                                                "source_id": source_id,
                                            });
                                            let _ = internal_index_store_broadcast.send(msg);
                                        }
                                        Err(e) => {
                                            // If indexing fails because file is gone (race condition), treat as removal
                                            if e.to_string().contains("No such file") {
                                                let kind = if rel_str.starts_with("memory/") {
                                                    "memory"
                                                } else if rel_str.starts_with("prompts/") {
                                                    "prompt"
                                                } else {
                                                    "note"
                                                };
                                                let _ = remove_internal_file(
                                                    &internal_index_store,
                                                    &source_id,
                                                    kind,
                                                    &rel_str,
                                                )
                                                .await;
                                                let msg = serde_json::json!({
                                                    "event": "file_removed",
                                                    "path": rel_str,
                                                    "source_id": source_id,
                                                });
                                                let _ = internal_index_store_broadcast.send(msg);
                                            } else {
                                                warn!(
                                                    "Watcher: failed to index {}: {}",
                                                    rel_str, e
                                                );
                                                // Still notify UI that *something* happened to this file
                                                let msg = serde_json::json!({
                                                    "event": "file_changed",
                                                    "path": rel_str,
                                                    "source_id": source_id,
                                                });
                                                let _ = internal_index_store_broadcast.send(msg);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => warn!("Watcher error for {}: {}", source_id, e),
            }
        }
    });

    Ok(())
}

/// Extract all metadata from markdown frontmatter as a JSON Value
fn extract_all_meta_from_content(content: &str) -> Option<serde_json::Value> {
    if !content.starts_with("---") {
        return None;
    }

    let mut parts = content.splitn(3, "---");
    parts.next(); // skip empty before first ---
    let frontmatter = parts.next()?;

    // Parse YAML into a generic serde_yaml::Value
    let yaml_val: serde_yaml::Value = serde_yaml::from_str(frontmatter).ok()?;

    // Convert YAML Value to JSON Value for storage in LanceDB/Metadata
    serde_json::to_value(yaml_val).ok()
}

/// Index a memory or prompt file into the internal index
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

    // Extract all metadata from frontmatter
    let frontmatter_json = extract_all_meta_from_content(&content);

    // Create unique document_id: {source_id}/{kind}/{relative_path}
    let document_id = format!("{}/{}/{}", source_id, kind, relative_path);

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

                    // Merge frontmatter into metadata
                    if let Some(meta) = &frontmatter_json {
                        if let Some(obj) = meta.as_object() {
                            for (k, v) in obj {
                                metadata[k] = v.clone();
                            }
                        }
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
                warn!("Failed to generate embeddings for {}: {}", relative_path, e);
                // Fallback: add chunks without embeddings
                for text_chunk in text_chunks {
                    chunks.push(Chunk {
                        id: Uuid::new_v4(),
                        source_id: source_id.to_string(),
                        document_id: document_id.clone(),
                        content: text_chunk,
                        embedding: None,
                        metadata: serde_json::json!({
                            "kind": kind,
                            "file_path": relative_path,
                        }),
                    });
                }
            }
        }
    }
    drop(model_guard);

    // Save to internal index store
    internal_index_store.add_chunks(chunks).await?;

    Ok(())
}

/// Remove a file from the internal index
pub async fn remove_internal_file(
    internal_index_store: &storage::InternalIndexStore,
    source_id: &str,
    kind: &str,
    relative_path: &str,
) -> Result<()> {
    // 1. Try deleting by path (default for notes/prompts)
    let document_id = format!("{}/{}/{}", source_id, kind, relative_path);
    internal_index_store.delete_document(&document_id).await?;

    info!("Removed internal file from index: {}", document_id);
    Ok(())
}

/// Rescan all internal files for a source
pub async fn rescan_internal_files(
    internal_index_store: &Arc<storage::InternalIndexStore>,
    embedding_model: &Arc<RwLock<Option<EmbeddingModel>>>,
    chunker: &Arc<TextChunker>,
    source_id: &str,
    source_root: &str,
) -> Result<(usize, usize)> {
    let root = Path::new(source_root);
    let mut files_indexed = 0;
    let mut files_failed = 0;

    // For "global" source, it's already the linggen dir.
    // For other sources, we check source_root/.linggen/
    let linggen_dir = if source_id == "global" {
        root.to_path_buf()
    } else {
        root.join(".linggen")
    };

    if !linggen_dir.exists() {
        return Ok((0, 0));
    }

    let subdirs = ["memory", "prompts", "notes"];
    for subdir in subdirs {
        let dir = linggen_dir.join(subdir);
        if !dir.exists() {
            continue;
        }

        let mut entries = tokio::fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(rel_path) = path.strip_prefix(&linggen_dir) {
                    let rel_str = rel_path.to_string_lossy().to_string();
                    match index_internal_file(
                        internal_index_store,
                        embedding_model,
                        chunker,
                        source_id,
                        &path,
                        &rel_str,
                    )
                    .await
                    {
                        Ok(_) => files_indexed += 1,
                        Err(e) => {
                            warn!("Failed to index {}: {}", rel_str, e);
                            files_failed += 1;
                        }
                    }
                }
            }
        }
    }

    Ok((files_indexed, files_failed))
}
