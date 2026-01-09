use serde::{Deserialize, Serialize};

/// User preferences for code generation and explanations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserPreferences {
    /// Style of explanation (e.g. "concise", "detailed", "bullet points")
    pub explanation_style: Option<String>,

    /// Preferred coding style (e.g. "functional", "OOP", "minimal")
    pub code_style: Option<String>,

    /// Documentation preference (e.g. "JSDoc", "inline comments")
    pub documentation_style: Option<String>,

    /// Testing preference (e.g. "unit tests", "TDD")
    pub test_style: Option<String>,

    /// Preferred programming language
    #[serde(alias = "preferred_language")]
    pub language_preference: Option<String>,

    /// Verbosity level ("concise", "balanced", "detailed")
    pub verbosity: Option<String>,
}

impl UserPreferences {
    /// Convert preferences to a human-readable prompt snippet
    pub fn to_prompt_instructions(&self) -> String {
        let mut instructions: Vec<String> = Vec::new();

        // Explanation Style
        if let Some(style) = &self.explanation_style {
            if !style.is_empty() {
                instructions.push(format!("Explanation style: {}", style));
            }
        }

        // Code Style
        if let Some(style) = &self.code_style {
            if !style.is_empty() {
                instructions.push(format!("Code style: {}", style));
            }
        }

        // Documentation Style
        if let Some(style) = &self.documentation_style {
            if !style.is_empty() {
                instructions.push(format!("Documentation style: {}", style));
            }
        }

        // Test Style
        if let Some(style) = &self.test_style {
            if !style.is_empty() {
                instructions.push(format!("Test style: {}", style));
            }
        }

        // Language Preference
        if let Some(lang) = &self.language_preference {
            if !lang.is_empty() {
                instructions.push(format!("Preferred language: {}", lang));
            }
        }

        // Verbosity
        if let Some(verbosity) = &self.verbosity {
            match verbosity.as_str() {
                "concise" => instructions.push("Be extremely concise".to_string()),
                "detailed" => {
                    instructions.push("Provide detailed, comprehensive output".to_string())
                }
                // balanced is default/neutral
                _ => {}
            }
        }

        if instructions.is_empty() {
            "Use default coding best practices.".to_string()
        } else {
            instructions.join(". ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_preferences() {
        let prefs = UserPreferences::default();
        assert!(prefs.explanation_style.is_none());
        assert_eq!(
            prefs.to_prompt_instructions(),
            "Use default coding best practices."
        );
    }

    #[test]
    fn test_prompt_instructions() {
        let prefs = UserPreferences {
            explanation_style: Some("bullet points".to_string()),
            code_style: Some("functional".to_string()),
            documentation_style: None,
            test_style: Some("TDD".to_string()),
            language_preference: Some("Rust".to_string()),
            verbosity: Some("concise".to_string()),
        };

        let instructions = prefs.to_prompt_instructions();
        assert!(instructions.contains("Explanation style: bullet points"));
        assert!(instructions.contains("Code style: functional"));
        assert!(instructions.contains("Test style: TDD"));
        assert!(instructions.contains("Preferred language: Rust"));
        assert!(instructions.contains("Be extremely concise"));
    }
}
