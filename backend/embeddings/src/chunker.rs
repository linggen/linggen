/// Text chunker for splitting documents into smaller pieces
pub struct TextChunker {
    chunk_size: usize,
    overlap: usize,
}

impl TextChunker {
    /// Create a new chunker with default settings (512 tokens, 128 overlap)
    pub fn new() -> Self {
        Self {
            chunk_size: 512,
            overlap: 128,
        }
    }

    /// Create a chunker with custom settings
    pub fn with_config(chunk_size: usize, overlap: usize) -> Self {
        Self {
            chunk_size,
            overlap,
        }
    }

    /// Split text into chunks
    pub fn chunk(&self, text: &str) -> Vec<String> {
        // Simple word-based chunking for now
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut chunks = Vec::new();

        if words.is_empty() {
            return chunks;
        }

        let mut start = 0;
        while start < words.len() {
            let end = (start + self.chunk_size).min(words.len());
            let chunk = words[start..end].join(" ");
            chunks.push(chunk);

            if end >= words.len() {
                break;
            }

            // Move start forward, accounting for overlap
            start = end - self.overlap;
            if start <= 0 {
                start = end;
            }
        }

        chunks
    }
}

impl Default for TextChunker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_chunking() {
        let chunker = TextChunker::with_config(5, 2);
        let text = "one two three four five six seven eight nine ten";
        let chunks = chunker.chunk(text);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], "one two three four five");
        assert_eq!(chunks[1], "four five six seven eight");
        assert_eq!(chunks[2], "seven eight nine ten");
    }

    #[test]
    fn test_no_overlap_needed() {
        let chunker = TextChunker::with_config(5, 0);
        let text = "one two three four five";
        let chunks = chunker.chunk(text);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "one two three four five");
    }

    #[test]
    fn test_empty_text() {
        let chunker = TextChunker::new();
        let chunks = chunker.chunk("");
        assert_eq!(chunks.len(), 0);
    }
}
