use anyhow::Result;
use embeddings::EmbeddingModel;
use rememberme_core::Chunk;
use rememberme_intent::{Intent, IntentClassifier};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use storage::{SourceProfile, UserPreferences, VectorStore};
use tracing::info;

pub mod profile_manager;
pub use profile_manager::ProfileManager;

/// Strategy for constructing the enhanced prompt
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PromptStrategy {
    /// Include full code chunks (default)
    FullCode,
    /// Include file paths and summaries only
    ReferenceOnly,
    /// Focus on high-level architecture
    Architectural,
}

/// Result of prompt enhancement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedPrompt {
    /// Original user query
    pub original_query: String,

    /// Enhanced prompt ready for AI assistant
    pub enhanced_prompt: String,

    /// Detected intent
    pub intent: Intent,

    /// Retrieved context chunks
    pub context_chunks: Vec<String>,

    /// Applied user preferences
    pub preferences_applied: bool,
}

use rememberme_llm::MiniLLM;
use tokio::sync::Mutex;

/// Prompt enhancer - orchestrates all stages
pub struct PromptEnhancer {
    intent_classifier: IntentClassifier,
    embedding_model: Arc<EmbeddingModel>,
    vector_store: Arc<VectorStore>,
}

impl PromptEnhancer {
    pub fn new(
        embedding_model: Arc<EmbeddingModel>,
        vector_store: Arc<VectorStore>,
        llm: Option<Arc<Mutex<MiniLLM>>>,
    ) -> Self {
        let intent_classifier = IntentClassifier::new(llm);

        Self {
            intent_classifier,
            embedding_model,
            vector_store,
        }
    }

    /// Enhance a user prompt through the pipeline
    pub async fn enhance(
        &mut self,
        query: &str,
        preferences: &UserPreferences,
        profile: &SourceProfile,
        strategy: PromptStrategy,
    ) -> Result<EnhancedPrompt> {
        // Stage 1: Intent Classification
        info!("Stage 1: Classifying intent...");
        let intent_result = self.intent_classifier.classify(query).await?;
        let intent = intent_result.intent.clone();

        // Stage 2: Context Retrieval (RAG)
        info!("Stage 2: Retrieving context via RAG...");
        let query_embedding = self.embedding_model.embed(query)?;
        let rag_chunks = self
            .vector_store
            .search(query_embedding, Some(query), 5)
            .await?;

        // Stage 3: Context Analysis (select top 3)
        info!("Stage 3: Analyzing context...");
        let top_chunks = if !rag_chunks.is_empty() {
            rag_chunks.into_iter().take(3).collect()
        } else {
            Vec::new()
        };

        // Stage 4: User Preferences
        info!("Stage 4: Applying user preferences...");
        let pref_instructions = preferences.to_prompt_instructions();

        // Stage 5: Prompt Enhancement
        info!(
            "Stage 5: Generating enhanced prompt with strategy {:?}...",
            strategy
        );
        let enhanced = self
            .generate_enhanced_prompt(
                query,
                &intent,
                &top_chunks,
                &pref_instructions,
                profile,
                strategy,
            )
            .await?;

        // Build result
        let chunk_contents: Vec<String> = top_chunks.iter().map(|c| c.content.clone()).collect();

        Ok(EnhancedPrompt {
            original_query: query.to_string(),
            enhanced_prompt: enhanced,
            intent,
            context_chunks: chunk_contents,
            preferences_applied: true,
        })
    }

    /// Generate the final enhanced prompt using a template-based approach
    async fn generate_enhanced_prompt(
        &mut self,
        query: &str,
        intent: &Intent,
        context: &[Chunk],
        preferences: &str,
        profile: &SourceProfile,
        strategy: PromptStrategy,
    ) -> Result<String> {
        // Format context based on strategy
        let context_text = if context.is_empty() {
            "No additional context available.".to_string()
        } else {
            match strategy {
                PromptStrategy::FullCode => context
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let file_info = if let Some(path) = c.metadata.get("file_path") {
                            format!("\nFile: {}", path.as_str().unwrap_or("Unknown"))
                        } else {
                            String::new()
                        };
                        format!(
                            "--- Context {} ---{}{}\n------------------",
                            i + 1,
                            file_info,
                            c.content
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                PromptStrategy::ReferenceOnly => context
                    .iter()
                    .map(|c| {
                        let path = c
                            .metadata
                            .get("file_path")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown");
                        format!("- {}", path)
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                PromptStrategy::Architectural => {
                    // For architectural, we might skip code chunks or just list files
                    context
                        .iter()
                        .map(|c| {
                            let path = c
                                .metadata
                                .get("file_path")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown");
                            format!("- {}", path)
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
        };

        // Construct structured prompt
        let profile_text = format!(
            "SOURCE PROFILE:\nName: {}\nDescription: {}\nTech Stack: {}\nArchitecture: {}\nConventions: {}",
            profile.name,
            profile.description,
            profile.tech_stack.join(", "),
            profile.architecture_notes.join(", "),
            profile.key_conventions.join(", ")
        );

        let enhanced = format!(
            "{}\n\nCONTEXT:\n{}\n\nUSER PREFERENCES:\n{}\n\nTASK: {:?}\n\nQUERY:\n{}",
            profile_text, context_text, preferences, intent, query
        );

        Ok(enhanced)
    }
}
