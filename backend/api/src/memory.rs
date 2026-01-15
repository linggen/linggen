use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use slug::slugify;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const FRONTMATTER_DELIM: &str = "---";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryScope {
    #[serde(default)]
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryCitation {
    pub source_id: String,
    pub file_path: String,
    #[serde(default)]
    pub line_range: Option<(i64, i64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMeta {
    #[serde(default)]
    pub id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub scope: MemoryScope,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub citations: Vec<MemoryCitation>,
}

#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub meta: MemoryMeta,
    pub body: String,
    pub path: PathBuf,
}

pub struct MemoryStore {
    root: PathBuf,
}

impl MemoryStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn memory_dir(&self) -> PathBuf {
        self.root.clone()
    }

    /// Load all memories (shallow scan, no cache).
    pub fn list(&self) -> Result<Vec<MemoryEntry>> {
        let mut entries = Vec::new();
        if !self.memory_dir().exists() {
            return Ok(entries);
        }

        for entry in fs::read_dir(self.memory_dir())? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            if let Ok(mem) = self.read_from_path(&path) {
                entries.push(mem);
            }
        }
        Ok(entries)
    }

    /// Simple search: substring match on title/body and optional tag/source filters.
    pub fn search(
        &self,
        query: Option<&str>,
        tags: Option<&[String]>,
        source_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<MemoryEntry>> {
        let q_lower = query.map(|q| q.to_lowercase());
        let tag_set: Option<Vec<String>> =
            tags.map(|t| t.iter().map(|s| s.to_lowercase()).collect());

        let mut results: Vec<MemoryEntry> = self
            .list()?
            .into_iter()
            .filter(|m| {
                if let Some(src) = source_id {
                    if !m.meta.scope.source_ids.is_empty()
                        && !m.meta.scope.source_ids.iter().any(|s| s == src)
                    {
                        return false;
                    }
                }

                if let Some(ts) = &tag_set {
                    let meta_tags: Vec<String> =
                        m.meta.tags.iter().map(|t| t.to_lowercase()).collect();
                    if !ts.iter().all(|t| meta_tags.contains(t)) {
                        return false;
                    }
                }

                if let Some(q) = &q_lower {
                    let hay = format!("{} {}", m.meta.title.to_lowercase(), m.body.to_lowercase());
                    hay.contains(q)
                } else {
                    true
                }
            })
            .collect();

        if let Some(max) = limit {
            results.truncate(max);
        }
        Ok(results)
    }

    pub fn read(&self, id: &str) -> Result<MemoryEntry> {
        // Try direct access first (new format: <id>.md)
        let direct_path = self.memory_dir().join(if id.ends_with(".md") {
            id.to_string()
        } else {
            format!("{}.md", id)
        });
        if direct_path.exists() {
            return self.read_from_path(&direct_path);
        }

        // Fallback to list search (search by title or id in metadata)
        let entries = self.list()?;
        entries
            .into_iter()
            .find(|m| {
                m.meta.id.as_deref() == Some(id)
                    || m.path.file_stem().and_then(|s| s.to_str()) == Some(id)
            })
            .ok_or_else(|| anyhow!("Memory not found: {}", id))
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        // Try direct access first
        let direct_path = self.memory_dir().join(if id.ends_with(".md") {
            id.to_string()
        } else {
            format!("{}.md", id)
        });
        if direct_path.exists() {
            fs::remove_file(direct_path)?;
            return Ok(());
        }

        let entries = self.list()?;
        if let Some(mem) = entries.into_iter().find(|m| {
            m.meta.id.as_deref() == Some(id)
                || m.path.file_stem().and_then(|s| s.to_str()) == Some(id)
        }) {
            fs::remove_file(mem.path)?;
            return Ok(());
        }
        Err(anyhow!("Memory not found: {}", id))
    }

    pub fn create(
        &self,
        title: &str,
        body: &str,
        tags: Vec<String>,
        scope: MemoryScope,
        citations: Vec<MemoryCitation>,
        confidence: Option<f64>,
    ) -> Result<MemoryEntry> {
        let now = Utc::now();
        let slug = Self::make_slug(title);
        let mut filename = format!("{}.md", slug);
        let mut path = self.memory_dir().join(&filename);

        // Avoid collision if title is the same
        if path.exists() {
            let uuid = Uuid::new_v4().to_string();
            let short_id = uuid.chars().take(4).collect::<String>();
            filename = format!("{}-{}.md", slug, short_id);
            path = self.memory_dir().join(filename);
        }

        let meta = MemoryMeta {
            id: None,
            title: title.to_string(),
            tags,
            scope,
            created_at: now,
            updated_at: now,
            confidence,
            citations,
        };

        self.write_file(&path, &meta, body)?;
        self.read_from_path(&path)
    }

    pub fn update(
        &self,
        id: &str,
        title: Option<&str>,
        body: Option<&str>,
        tags: Option<Vec<String>>,
        scope: Option<MemoryScope>,
        citations: Option<Vec<MemoryCitation>>,
        confidence: Option<Option<f64>>,
    ) -> Result<MemoryEntry> {
        let mem = self.read(id)?;
        let mut meta = mem.meta.clone();

        // Gradually remove ID from files as they are updated
        meta.id = None;

        if let Some(t) = title {
            meta.title = t.to_string();
        }
        if let Some(ts) = tags {
            meta.tags = ts;
        }
        if let Some(sc) = scope {
            meta.scope = sc;
        }
        if let Some(cits) = citations {
            meta.citations = cits;
        }
        if let Some(conf) = confidence {
            meta.confidence = conf;
        }
        meta.updated_at = Utc::now();

        let new_body = body.unwrap_or(&mem.body).to_string();

        self.write_file(&mem.path, &meta, &new_body)?;
        self.read_from_path(&mem.path)
    }

    fn write_file(&self, path: &Path, meta: &MemoryMeta, body: &str) -> Result<()> {
        let mut file = fs::File::create(path)?;
        let front = serde_yaml::to_string(meta)?;
        writeln!(file, "{}", FRONTMATTER_DELIM)?;
        write!(file, "{}", front)?;
        writeln!(file, "{}", FRONTMATTER_DELIM)?;
        writeln!(file)?;
        write!(file, "{}", body)?;
        Ok(())
    }

    pub fn read_from_path(&self, path: &Path) -> Result<MemoryEntry> {
        let mut content = String::new();
        fs::File::open(path)?.read_to_string(&mut content)?;

        // Parse frontmatter
        let mut parts = content.splitn(3, FRONTMATTER_DELIM);
        let _leading = parts.next(); // empty before first ---
        let front = parts.next().ok_or_else(|| anyhow!("Missing frontmatter"))?;
        let body = parts
            .next()
            .ok_or_else(|| anyhow!("Missing body after frontmatter"))?;

        let meta: MemoryMeta = serde_yaml::from_str(front)?;

        // Trim leading newlines from body
        let body = body.trim_start().to_string();

        Ok(MemoryEntry {
            meta,
            body,
            path: path.to_path_buf(),
        })
    }

    /// Utility: sanitize slug (fallback) if needed
    pub fn make_slug(title: &str) -> String {
        let slug = slugify(title);
        let re = Regex::new(r"[^a-zA-Z0-9_-]").unwrap();
        let cleaned = re.replace_all(&slug, "-");
        cleaned.to_string()
    }
}
