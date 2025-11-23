use anyhow::Result;
use serde::{Deserialize, Serialize};

/// User's preferred explanation style
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExplanationStyle {
    /// Brief, to-the-point explanations
    Concise,
    /// Detailed with examples
    Detailed,
    /// Step-by-step breakdown
    StepByStep,
}

impl Default for ExplanationStyle {
    fn default() -> Self {
        Self::Concise
    }
}

/// User's preferred output format
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// Bullet points
    Bullets,
    /// Prose/paragraph format
    Prose,
    /// Code with inline comments
    CodeWithComments,
    /// Code diff format
    CodeDiff,
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Bullets
    }
}

/// User preferences for code generation and explanations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreferences {
    /// How verbose should explanations be
    pub explanation_style: ExplanationStyle,

    /// Preferred output format
    pub output_format: OutputFormat,

    /// Preferred programming language (when applicable)
    pub preferred_language: Option<String>,

    /// Whether to include examples in explanations
    pub include_examples: bool,

    /// Whether to show related code/files
    pub show_related_code: bool,

    /// Maximum explanation length (words)
    pub max_explanation_words: usize,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            explanation_style: ExplanationStyle::Concise,
            output_format: OutputFormat::Bullets,
            preferred_language: None,
            include_examples: true,
            show_related_code: true,
            max_explanation_words: 300,
        }
    }
}

impl UserPreferences {
    /// Convert preferences to a human-readable prompt snippet
    pub fn to_prompt_instructions(&self) -> String {
        let mut instructions: Vec<String> = Vec::new();

        // Style
        match self.explanation_style {
            ExplanationStyle::Concise => {
                instructions.push("Keep explanations concise and to-the-point".to_string())
            }
            ExplanationStyle::Detailed => {
                instructions.push("Provide detailed explanations with context".to_string())
            }
            ExplanationStyle::StepByStep => {
                instructions.push("Break down explanations step-by-step".to_string())
            }
        }

        // Format
        match self.output_format {
            OutputFormat::Bullets => {
                instructions.push("Format output as bullet points".to_string())
            }
            OutputFormat::Prose => instructions.push("Use prose/paragraph format".to_string()),
            OutputFormat::CodeWithComments => {
                instructions.push("Show code with inline comments".to_string())
            }
            OutputFormat::CodeDiff => instructions.push("Show changes as code diffs".to_string()),
        }

        // Examples
        if self.include_examples {
            instructions.push("Include examples when helpful".to_string());
        } else {
            instructions.push("Skip examples unless critical".to_string());
        }

        // Related code
        if self.show_related_code {
            instructions.push("Mention related code/modules".to_string());
        }

        // Language
        if let Some(lang) = &self.preferred_language {
            instructions.push(format!("Prefer {} when showing code", lang));
        }

        instructions.join(". ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_preferences() {
        let prefs = UserPreferences::default();
        assert_eq!(prefs.explanation_style, ExplanationStyle::Concise);
        assert_eq!(prefs.output_format, OutputFormat::Bullets);
        assert!(prefs.include_examples);
    }

    #[test]
    fn test_prompt_instructions() {
        let prefs = UserPreferences {
            explanation_style: ExplanationStyle::Concise,
            output_format: OutputFormat::CodeDiff,
            preferred_language: Some("Rust".to_string()),
            include_examples: false,
            show_related_code: true,
            max_explanation_words: 200,
        };

        let instructions = prefs.to_prompt_instructions();
        assert!(instructions.contains("concise"));
        assert!(instructions.contains("code diff"));
        assert!(instructions.contains("Rust"));
    }
}
