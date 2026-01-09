use anyhow::Result;
use linggen_llm::MiniLLM;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::error;

/// Developer intent types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// Fix a bug or error
    FixBug,
    /// Explain code or concepts
    ExplainCode,
    /// Refactor existing code
    RefactorCode,
    /// Write tests
    WriteTest,
    /// Debug an error
    DebugError,
    /// Generate documentation
    GenerateDoc,
    /// Analyze performance
    AnalyzePerformance,
    /// Ask a general question
    AskQuestion,
    /// Other/unclear intent
    Other(String),
}

impl fmt::Display for Intent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Intent::FixBug => write!(f, "Fix a bug or error"),
            Intent::ExplainCode => write!(f, "Explain code or concepts"),
            Intent::RefactorCode => write!(f, "Refactor existing code"),
            Intent::WriteTest => write!(f, "Write tests"),
            Intent::DebugError => write!(f, "Debug an error"),
            Intent::GenerateDoc => write!(f, "Generate documentation"),
            Intent::AnalyzePerformance => write!(f, "Analyze performance"),
            Intent::AskQuestion => write!(f, "Ask a general question"),
            Intent::Other(s) => write!(f, "Other: {}", s),
        }
    }
}

impl Intent {
    /// Parse intent from string
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().trim() {
            "fix_bug" | "fixbug" | "bug" => Intent::FixBug,
            "explain_code" | "explaincode" | "explain" => Intent::ExplainCode,
            "refactor_code" | "refactorcode" | "refactor" => Intent::RefactorCode,
            "write_test" | "writetest" | "test" => Intent::WriteTest,
            "debug_error" | "debugerror" | "debug" => Intent::DebugError,
            "generate_doc" | "generatedoc" | "doc" | "documentation" => Intent::GenerateDoc,
            "analyze_performance" | "analyzeperformance" | "perf" | "performance" => {
                Intent::AnalyzePerformance
            }
            "ask_question" | "askquestion" | "question" => Intent::AskQuestion,
            other => Intent::Other(other.to_string()),
        }
    }
}

/// Result of intent classification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentResult {
    /// Detected intent
    pub intent: Intent,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f32,
    /// Key entities extracted from query
    pub entities: Vec<String>,
    /// Whether context retrieval is needed
    pub needs_context: bool,
}

/// Intent classifier with LLM support and heuristic fallback
pub struct IntentClassifier {
    llm: Option<Arc<Mutex<MiniLLM>>>,
}

impl IntentClassifier {
    pub fn new(llm: Option<Arc<Mutex<MiniLLM>>>) -> Self {
        Self { llm }
    }

    /// Classify developer query intent
    pub async fn classify(&mut self, query: &str) -> Result<IntentResult> {
        // Try LLM first if available
        if let Some(llm) = &self.llm {
            match self.classify_with_llm(llm, query).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    error!(
                        "LLM intent classification failed: {}. Falling back to heuristics.",
                        e
                    );
                }
            }
        }

        // Fallback to heuristics
        self.classify_heuristic(query)
    }

    async fn classify_with_llm(
        &self,
        llm: &Arc<Mutex<MiniLLM>>,
        query: &str,
    ) -> Result<IntentResult> {
        let system_prompt = r#"You are an intent classifier for a coding assistant.
Analyze the user's query and classify it into one of these intents:
- fix_bug: Fix a bug or error
- explain_code: Explain code or concepts
- refactor_code: Refactor existing code
- write_test: Write tests
- debug_error: Debug an error
- generate_doc: Generate documentation
- analyze_performance: Analyze performance
- ask_question: Ask a general question
- other: Any other intent (specify briefly)

Also extract key entities (file names, function names, concepts) and determine if context is needed.

Output JSON only:
{
  "intent": "intent_name",
  "other_description": "optional description if intent is other",
  "confidence": 0.9,
  "entities": ["entity1", "entity2"],
  "needs_context": true
}"#;

        let mut llm = llm.lock().await;
        let response = llm.generate_with_system(system_prompt, query, 200).await?;

        // Parse JSON response
        // Clean up markdown code blocks if present
        let json_str = response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        #[derive(Deserialize)]
        struct LLMResponse {
            intent: String,
            other_description: Option<String>,
            confidence: f32,
            entities: Vec<String>,
            needs_context: bool,
        }

        let parsed: LLMResponse = serde_json::from_str(json_str)?;

        let intent = if parsed.intent == "other" {
            Intent::Other(
                parsed
                    .other_description
                    .unwrap_or_else(|| "Unknown".to_string()),
            )
        } else {
            Intent::from_str(&parsed.intent)
        };

        Ok(IntentResult {
            intent,
            confidence: parsed.confidence,
            entities: parsed.entities,
            needs_context: parsed.needs_context,
        })
    }

    fn classify_heuristic(&self, query: &str) -> Result<IntentResult> {
        let q = query.to_lowercase();

        let intent = if q.contains("bug") || q.contains("fix") || q.contains("error") {
            Intent::FixBug
        } else if q.contains("explain") || q.contains("what is") || q.contains("how does") {
            Intent::ExplainCode
        } else if q.contains("refactor") || q.contains("clean up") {
            Intent::RefactorCode
        } else if q.contains("test") || q.contains("unit test") {
            Intent::WriteTest
        } else if q.contains("doc") || q.contains("documentation") {
            Intent::GenerateDoc
        } else if q.contains("perf") || q.contains("optimize") || q.contains("slow") {
            Intent::AnalyzePerformance
        } else {
            Intent::AskQuestion
        };

        // Very rough entity extraction: split on whitespace and keep alphanumeric words.
        let entities: Vec<String> = q
            .split_whitespace()
            .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(IntentResult {
            intent,
            confidence: 0.8,
            entities,
            needs_context: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_parsing() {
        assert_eq!(Intent::from_str("fix_bug"), Intent::FixBug);
        assert_eq!(Intent::from_str("explain"), Intent::ExplainCode);
        assert_eq!(Intent::from_str("test"), Intent::WriteTest);
    }

    #[tokio::test]
    async fn test_classifier_heuristic() {
        let mut classifier = IntentClassifier::new(None);
        let result = classifier.classify("fix auth timeout bug").await.unwrap();

        assert_eq!(result.intent, Intent::FixBug);
    }
}
