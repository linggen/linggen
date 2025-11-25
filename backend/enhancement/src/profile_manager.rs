use anyhow::Result;
use rememberme_core::Chunk;
use rememberme_llm::MiniLLM;
use std::path::PathBuf;
use std::sync::Arc;
use storage::SourceProfile;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

pub struct ProfileManager {
    llm: Option<Arc<Mutex<MiniLLM>>>,
}

impl ProfileManager {
    pub fn new(llm: Option<Arc<Mutex<MiniLLM>>>) -> Self {
        Self { llm }
    }

    /// Generate initial profile by scanning key chunks (grouped by file)
    pub async fn generate_initial_profile(&self, chunks: Vec<Chunk>) -> Result<SourceProfile> {
        // Group chunks by file path and assemble per-file content
        let mut files: Vec<(PathBuf, String)> = Vec::new();
        use std::collections::HashMap;
        let mut file_map: HashMap<String, String> = HashMap::new();

        for chunk in &chunks {
            let file_key = chunk
                .metadata
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or(&chunk.document_id)
                .to_string();
            let entry = file_map.entry(file_key).or_insert_with(String::new);
            if !entry.is_empty() {
                entry.push_str("\n\n");
            }
            entry.push_str(&chunk.content);
            // Log embedding length for debugging
            info!(
                "Chunk {} embedding length: {}",
                chunk.document_id,
                chunk.embedding.as_ref().map(|e| e.len()).unwrap_or(0)
            );
        }

        for (path_str, content) in file_map {
            files.push((PathBuf::from(path_str), content));
        }

        if let Some(llm) = &self.llm {
            info!(
                "ProfileManager: grouped {} chunks into {} files for profile generation. First 3 files: {:?}",
                chunks.len(),
                files.len(),
                files
                    .iter()
                    .take(3)
                    .map(|(p, _)| p.display().to_string())
                    .collect::<Vec<_>>()
            );

            // Log a quick summary of the first few chunks for debugging
            if let Some(first) = chunks.first() {
                let file_path = first
                    .metadata
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&first.document_id);
                let snippet: String = first.content.chars().take(100).collect();
                let emb_len = first.embedding.as_ref().map(|e| e.len()).unwrap_or(0);

                info!(
                    "Generating initial profile from {} chunks. Example chunk -> file: {}, emb_len: {}, snippet: {:?}",
                    chunks.len(),
                    file_path,
                    emb_len,
                    snippet
                );
            } else {
                info!("Generating initial profile but no chunks were provided");
            }

            // Prepare context from files (limit to avoid overwhelming smaller models)
            let file_context = files
                .iter()
                .map(|(path, content)| {
                    // Truncate by characters (not bytes) to avoid splitting UTF-8 code points
                    let snippet: String = if content.chars().count() > 1000 {
                        let mut s: String = content.chars().take(1000).collect();
                        s.push_str("... (truncated)");
                        s
                    } else {
                        content.clone()
                    };

                    format!(
                        "File: {}\nContent:\n{}\n------------------",
                        path.display(),
                        snippet
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");

            // Limit total context so the local model doesn't get overwhelmed.
            // 6k characters is usually enough to cover key docs without causing
            // long, unstable generations.
            let context_limit = 1000;
            let limited_context = if file_context.len() > context_limit {
                format!("{}... (truncated)", &file_context[..context_limit])
            } else {
                file_context
            };

            let system_prompt = r#"You are a senior software architect.
Analyze the provided source files and write a clear, plain-text PROJECT PROFILE.

Write a single narrative that covers:
- Project name and a one-line summary
- A short description of what the project does
- The main technologies and languages used
- High-level architecture or structure (if visible)
- Any notable conventions or patterns

IMPORTANT:
- Keep it concise but informative (aim for 2–6 short paragraphs)."#;

            let user_prompt = format!(
                "Analyze these files and write the project profile described above:\n\n{}",
                limited_context
            );

            info!("ProfileManager: user prompt: {}", user_prompt);

            // Try LLM generation with error handling
            match async {
                info!("ProfileManager: invoking LLM to generate profile...");
                let mut llm = llm.lock().await;
                llm.generate_with_system(system_prompt, &user_prompt, 800)
                    .await
            }
            .await
            {
                Ok(response) => {
                    let text = response.trim();
                    if !text.is_empty() {
                        let profile = SourceProfile {
                            profile_name: "Generated Profile".to_string(),
                            description: text.to_string(),
                        };
                        info!(
                            "ProfileManager: LLM-generated plain-text profile {}",
                            profile.description
                        );
                        return Ok(profile);
                    } else {
                        warn!("ProfileManager: LLM returned empty profile text. Falling back to default profile.");
                    }
                }
                Err(e) => {
                    error!(
                        "LLM generation failed: {}. Falling back to basic file parsing",
                        e
                    );
                }
            }
        }

        // Fallback: no LLM profile available – build a simple profile directly
        // from the indexed files so the user still gets something useful.
        if !files.is_empty() {
            // Prefer README-like files for a human-friendly summary.
            let readme_candidate = files.iter().find(|(path, _)| {
                if let Some(name) = path.file_name() {
                    let name = name.to_string_lossy().to_lowercase();
                    name == "readme.md" || name == "readme" || name.starts_with("readme.")
                } else {
                    false
                }
            });

            let (profile_name, description) = if let Some((path, content)) = readme_candidate {
                // Use the first few lines of the README as a basic description.
                let first_lines: String = content.lines().take(12).collect::<Vec<_>>().join("\n");
                (
                    format!("Basic Profile from {}", path.display()),
                    first_lines,
                )
            } else {
                // Fall back to the first file we saw.
                let (path, content) = &files[0];
                let first_lines: String = content.lines().take(12).collect::<Vec<_>>().join("\n");
                (
                    format!("Basic Profile from {}", path.display()),
                    first_lines,
                )
            };

            let profile = SourceProfile {
                profile_name,
                description,
            };

            info!("ProfileManager: generated basic fallback profile from files");
            return Ok(profile);
        }

        // No files available at all – return an empty profile.
        info!(
            "ProfileManager: no files available; returning default empty profile for manual editing"
        );
        Ok(SourceProfile::default())
    }

    /// Update profile based on user query (learning loop)
    pub async fn update_profile_from_query(
        &self,
        query: &str,
        current_profile: &SourceProfile,
    ) -> Result<Option<SourceProfile>> {
        if let Some(llm) = &self.llm {
            let system_prompt = r#"You are a project manager maintaining a project profile.
Analyze the user's query to see if it reveals NEW information about the project's architecture, tech stack, or design that is NOT already in the profile.
If yes, extract the new information and return the UPDATED profile JSON.
If no new information is found, return "NO_UPDATE".

Current Profile:
"#;
            let current_json = serde_json::to_string_pretty(current_profile)?;

            let full_system_prompt = format!("{}{}", system_prompt, current_json);

            let user_prompt = format!("User Query: {}", query);

            let mut llm = llm.lock().await;
            let response = llm
                .generate_with_system(&full_system_prompt, &user_prompt, 1000)
                .await?;

            let response = response.trim();
            if response.contains("NO_UPDATE") {
                return Ok(None);
            }

            // Parse JSON
            let json_str = response
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();

            match serde_json::from_str::<SourceProfile>(json_str) {
                Ok(updated_profile) => {
                    info!("Profile updated from user query");
                    Ok(Some(updated_profile))
                }
                Err(e) => {
                    error!("Failed to parse updated profile JSON: {}", e);
                    Ok(None)
                }
            }
        } else {
            Ok(None)
        }
    }
}
