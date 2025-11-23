use anyhow::Result;
use embeddings::EmbeddingModel;
use rememberme_context::ContextAnalyzer;
use rememberme_core::Chunk;
use rememberme_intent::{Intent, IntentClassifier};
use rememberme_llm::MiniLLM;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use storage::{UserPreferences, VectorStore};

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

/// Prompt enhancer - orchestrates all 5 stages
pub struct PromptEnhancer {
    llm: Arc<MiniLLM>,
    intent_classifier: IntentClassifier,
    context_analyzer: ContextAnalyzer,
    embedding_model: Arc<EmbeddingModel>,
    vector_store: Arc<VectorStore>,
}

impl PromptEnhancer {
    pub fn new(
        llm: Arc<MiniLLM>,
        embedding_model: Arc<EmbeddingModel>,
        vector_store: Arc<VectorStore>,
    ) -> Self {
        let intent_classifier = IntentClassifier::new(llm.clone());
        let context_analyzer = ContextAnalyzer::new(llm.clone());

        Self {
            llm,
            intent_classifier,
            context_analyzer,
            embedding_model,
            vector_store,
        }
    }

    /// Enhance a user prompt through the full 5-stage pipeline
    pub async fn enhance(
        &mut self,
        query: &str,
        preferences: &UserPreferences,
    ) -> Result<EnhancedPrompt> {
        // Stage 1: Intent Classification
        println!("Stage 1: Classifying intent...");
        let intent_result = self.intent_classifier.classify(query).await?;
        let intent = intent_result.intent.clone();

        // Stage 2: Context Retrieval (RAG)
        println!("Stage 2: Retrieving context via RAG...");
        let query_embedding = self.embedding_model.embed(query)?;
        let rag_chunks = self
            .vector_store
            .search(query_embedding, Some(query), 5)
            .await?;

        // Stage 3: Context Analysis (select top 2-3)
        println!("Stage 3: Analyzing context...");
        let top_chunks = if !rag_chunks.is_empty() {
            self.context_analyzer
                .select_relevant(query, rag_chunks, 3)
                .await?
        } else {
            Vec::new()
        };

        // Stage 4: User Preferences (already loaded, passed in)
        println!("Stage 4: Applying user preferences...");
        let pref_instructions = preferences.to_prompt_instructions();

        // Stage 5: Prompt Enhancement
        println!("Stage 5: Generating enhanced prompt...");
        let enhanced = self
            .generate_enhanced_prompt(query, &intent, &top_chunks, &pref_instructions)
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

    /// Generate the final enhanced prompt using LLM
    async fn generate_enhanced_prompt(
        &mut self,
        query: &str,
        intent: &Intent,
        context: &[Chunk],
        preferences: &str,
    ) -> Result<String> {
        let system_prompt = "You are a prompt enhancement assistant for developers. \
                            Your job is to take a simple query and enhance it with context \
                            and preferences to get the best response from an AI coding assistant.";

        // Format context
        let context_text = if context.is_empty() {
            "No additional context available.".to_string()
        } else {
            context
                .iter()
                .enumerate()
                .map(|(i, c)| format!("Context {}:\n{}", i + 1, c.content))
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        let user_prompt = format!(
            r#"Original Query: "{}"

Intent: {:?}

Available Context:
{}

User Preferences: {}

Task: Create an enhanced version of the original query that:
1. Incorporates the relevant context
2. Follows the user's preferences
3. Is clear and actionable for an AI coding assistant
4. Maintains the original intent

Enhanced Query:"#,
            query, intent, context_text, preferences
        );

        let llm = Arc::get_mut(&mut self.llm)
            .ok_or_else(|| anyhow::anyhow!("Failed to get mutable LLM reference"))?;

        let enhanced = llm
            .generate_with_system(&system_prompt, &user_prompt, 300)
            .await?;

        Ok(enhanced.trim().to_string())
    }
}
