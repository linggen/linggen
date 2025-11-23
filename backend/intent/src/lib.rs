use anyhow::Result;
use rememberme_llm::MiniLLM;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
            "analyze_performance" | "analyzeperformance" | "perf" | "performance" => Intent::AnalyzePerformance,
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

/// Intent classifier using Mini LLM
pub struct IntentClassifier {
    llm: Arc<MiniLLM>,
}

impl IntentClassifier {
    /// Create a new intent classifier
    pub fn new(llm: Arc<MiniLLM>) -> Self {
        Self { llm }
    }
    
    /// Classify developer query intent
    pub async fn classify(&mut self, query: &str) -> Result<IntentResult> {
        let system_prompt = "You are an intent classification assistant for developer queries. \
                            Analyze the query and respond ONLY with valid JSON.";
        
        let user_prompt = format!(
            r#"Classify this developer query:
Query: "{}"

Respond with JSON in this exact format:
{{
  "intent": "fix_bug|explain_code|refactor_code|write_test|debug_error|generate_doc|analyze_performance|ask_question|other",
  "confidence": 0.95,
  "entities": ["entity1", "entity2"],
  "needs_context": true
}}

Intent options:
- fix_bug: fixing bugs or errors
- explain_code: explaining code or concepts
- refactor_code: refactoring or improving code structure
- write_test: writing unit/integration tests
- debug_error: debugging runtime errors
- generate_doc: generating documentation
- analyze_performance: analyzing or optimizing performance
- ask_question: general questions
- other: unclear intent

Entities: key technical terms (e.g., "auth", "timeout", "redis")
Needs context: true if code/docs would help answer the query

Respond ONLY with valid JSON, no explanation:"#,
            query
        );
        
        // Get LLM response
        let llm = Arc::get_mut(&mut self.llm)
            .ok_or_else(|| anyhow::anyhow!("Failed to get mutable LLM reference"))?;
        
        let response = llm
            .generate_with_system(&system_prompt, &user_prompt, 200)
            .await?;
        
        // Parse JSON response
        let result = self.parse_response(&response)?;
        
        Ok(result)
    }
    
    /// Parse LLM response to IntentResult
    fn parse_response(&self, response: &str) -> Result<IntentResult> {
        // Find JSON in response (LLM might add extra text)
        let json_start = response.find('{').unwrap_or(0);
        let json_end = response.rfind('}').map(|i| i + 1).unwrap_or(response.len());
        let json_str = &response[json_start..json_end];
        
        // Parse JSON
        let parsed: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse JSON: {}. Response: {}", e, response))?;
        
        // Extract fields
        let intent_str = parsed["intent"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'intent' field"))?;
        
        let intent = Intent::from_str(intent_str);
        
        let confidence = parsed["confidence"]
            .as_f64()
            .unwrap_or(0.5) as f32;
        
        let entities: Vec<String> = parsed["entities"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        
        let needs_context = parsed["needs_context"]
            .as_bool()
            .unwrap_or(true);
        
        Ok(IntentResult {
            intent,
            confidence,
            entities,
            needs_context,
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
    
    #[test]
    fn test_json_parsing() {
        let classifier = IntentClassifier {
            llm: Arc::new(MiniLLM::new(Default::default()).unwrap()),
        };
        
        let json = r#"{"intent": "fix_bug", "confidence": 0.95, "entities": ["auth", "timeout"], "needs_context": true}"#;
        let result = classifier.parse_response(json).unwrap();
        
        assert_eq!(result.intent, Intent::FixBug);
        assert_eq!(result.confidence, 0.95);
        assert_eq!(result.entities, vec!["auth", "timeout"]);
        assert!(result.needs_context);
    }
}
