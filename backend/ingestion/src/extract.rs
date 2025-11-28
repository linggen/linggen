//! Text extraction from various file formats (PDF, DOCX, plain text)

use dotext::MsDoc;
use std::io::Read;
use std::path::Path;

/// Extract text content from a file based on its extension.
///
/// Supports:
/// - PDF files (.pdf)
/// - Word documents (.docx)
/// - Plain text files (all other extensions)
///
/// Returns `None` if the file cannot be read or parsed.
pub fn extract_text(path: &Path) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "pdf" => extract_pdf(path),
        "docx" => extract_docx(path),
        // Plain text files (code, markdown, config, etc.)
        _ => std::fs::read_to_string(path).ok(),
    }
}

/// Extract text from a PDF file
fn extract_pdf(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    match pdf_extract::extract_text_from_mem(&bytes) {
        Ok(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                tracing::debug!("PDF file has no extractable text: {:?}", path);
                None
            } else {
                Some(text)
            }
        }
        Err(e) => {
            tracing::warn!("Failed to extract text from PDF {:?}: {}", path, e);
            None
        }
    }
}

/// Extract text from a DOCX file
fn extract_docx(path: &Path) -> Option<String> {
    let mut file = match dotext::Docx::open(path) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("Failed to open DOCX {:?}: {}", path, e);
            return None;
        }
    };

    let mut text = String::new();
    match file.read_to_string(&mut text) {
        Ok(_) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                tracing::debug!("DOCX file has no extractable text: {:?}", path);
                None
            } else {
                Some(text)
            }
        }
        Err(e) => {
            tracing::warn!("Failed to read DOCX {:?}: {}", path, e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_extract_plain_text() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "Hello, world!").unwrap();

        let text = extract_text(&file_path);
        assert!(text.is_some());
        assert!(text.unwrap().contains("Hello, world!"));
    }

    #[test]
    fn test_extract_unknown_extension_as_text() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");

        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "fn main() {{}}").unwrap();

        let text = extract_text(&file_path);
        assert!(text.is_some());
        assert!(text.unwrap().contains("fn main()"));
    }
}
