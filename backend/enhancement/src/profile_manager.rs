use anyhow::Result;
use rememberme_llm::MiniLLM;
use std::path::PathBuf;
use std::sync::Arc;
use storage::SourceProfile;
use tokio::sync::Mutex;
use tracing::{error, info};

pub struct ProfileManager {
    llm: Option<Arc<Mutex<MiniLLM>>>,
}

impl ProfileManager {
    pub fn new(llm: Option<Arc<Mutex<MiniLLM>>>) -> Self {
        Self { llm }
    }

    /// Generate initial profile by scanning key files
    pub async fn generate_initial_profile(
        &self,
        files: Vec<(PathBuf, String)>,
    ) -> Result<SourceProfile> {
        if let Some(llm) = &self.llm {
            info!("Generating initial profile from {} files...", files.len());

            // Prepare context from files (limit to avoid shape mismatch errors)
            let file_context = files
                .iter()
                .map(|(path, content)| {
                    format!(
                        "File: {}\nContent:\n{}\n------------------",
                        path.display(),
                        if content.len() > 1000 {
                            format!("{}... (truncated)", &content[..1000])
                        } else {
                            content.clone()
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");

            // Limit total context to prevent shape mismatch
            let context_limit = 3000;
            let limited_context = if file_context.len() > context_limit {
                format!("{}... (truncated)", &file_context[..context_limit])
            } else {
                file_context
            };

            let system_prompt = r#"You are a senior software architect.
Analyze the provided source files to extract a project profile.
Identify the project name, description, tech stack (languages, frameworks, libraries), architectural patterns, and coding conventions.

Output JSON only:
{
  "name": "Project Name",
  "description": "Brief description",
  "tech_stack": ["Rust", "React", "Axum"],
  "architecture_notes": ["Microservices", "Clean Architecture"],
  "key_conventions": ["Use snake_case", "Prefer composition"]
}"#;

            let user_prompt = format!("Analyze these files:\n\n{}", limited_context);

            // Try LLM generation with error handling
            match async {
                let mut llm = llm.lock().await;
                llm.generate_with_system(system_prompt, &user_prompt, 800)
                    .await
            }
            .await
            {
                Ok(response) => {
                    // Parse JSON
                    let json_str = response
                        .trim()
                        .trim_start_matches("```json")
                        .trim_start_matches("```")
                        .trim_end_matches("```")
                        .trim();

                    match serde_json::from_str::<SourceProfile>(json_str) {
                        Ok(profile) => {
                            info!("Initial profile generated successfully: {}", profile.name);
                            return Ok(profile);
                        }
                        Err(e) => {
                            error!("Failed to parse LLM response as JSON: {}", e);
                            info!("Falling back to basic file parsing");
                        }
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

        // Fallback: basic parsing from file contents
        info!("Using basic file parsing for profile generation");
        self.generate_profile_from_files(&files)
    }

    /// Simple fallback profile generation from common files
    fn generate_profile_from_files(&self, files: &[(PathBuf, String)]) -> Result<SourceProfile> {
        let mut profile = SourceProfile::default();

        for (path, content) in files {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            // Extract from Cargo.toml
            if filename == "Cargo.toml" {
                if let Some(name) = content.lines().find(|l| l.starts_with("name =")) {
                    profile.name = name
                        .split('=')
                        .nth(1)
                        .unwrap_or("")
                        .trim()
                        .trim_matches('"')
                        .to_string();
                }
                if content.contains("tokio") {
                    profile.tech_stack.push("Tokio".to_string());
                }
                if content.contains("axum") {
                    profile.tech_stack.push("Axum".to_string());
                }
                if content.contains("serde") {
                    profile.tech_stack.push("Serde".to_string());
                }
                profile.tech_stack.push("Rust".to_string());
            }

            // Extract from package.json
            if filename == "package.json" {
                if content.contains("react") {
                    profile.tech_stack.push("React".to_string());
                }
                if content.contains("typescript") {
                    profile.tech_stack.push("TypeScript".to_string());
                }
                if content.contains("vue") {
                    profile.tech_stack.push("Vue".to_string());
                }
                profile.tech_stack.push("JavaScript/TypeScript".to_string());
            }

            // Extract from README
            if filename.to_lowercase().starts_with("readme") {
                let first_line = content.lines().next().unwrap_or("");
                if first_line.starts_with('#') {
                    profile.name = first_line.trim_start_matches('#').trim().to_string();
                }
                let first_para = content.lines().take(10).collect::<Vec<_>>().join(" ");
                if profile.description.is_empty() && first_para.len() > 10 {
                    profile.description = first_para.chars().take(200).collect();
                }
            }
        }

        if profile.name.is_empty() {
            profile.name = "Source Project".to_string();
        }
        if profile.description.is_empty() {
            profile.description = "Auto-discovered source".to_string();
        }

        Ok(profile)
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
