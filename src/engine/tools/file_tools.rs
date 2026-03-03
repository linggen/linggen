use super::tool_helpers::{build_globset, sanitize_rel_path, to_rel_string};
use super::{is_rfc1918_172, ToolResult, Tools};
use anyhow::Result;
use ignore::WalkBuilder;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tracing::info;

#[derive(Debug, Deserialize)]
pub(super) struct ListFilesArgs {
    pub(super) globs: Option<Vec<String>>,
    pub(super) max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReadFileArgs {
    #[serde(alias = "file", alias = "filepath")]
    pub(super) path: String,
    pub(super) max_bytes: Option<usize>,
    pub(super) line_range: Option<[usize; 2]>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CaptureScreenshotArgs {
    pub(super) url: String,
    pub(super) delay_ms: Option<u64>,
}

impl Tools {
    pub(super) fn list_files(&self, args: ListFilesArgs) -> Result<ToolResult> {
        let globset = build_globset(args.globs.as_deref())?;
        let max_results = args.max_results.unwrap_or(200);

        let mut out = Vec::new();
        let walker = WalkBuilder::new(&self.root)
            .standard_filters(true)
            .hidden(true)
            .build();

        for entry in walker {
            let entry = match entry {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            let rel = to_rel_string(&self.root, path)?;
            if let Some(gs) = &globset {
                if !gs.is_match(Path::new(&rel)) {
                    continue;
                }
            }
            out.push(rel);
            if out.len() >= max_results {
                break;
            }
        }

        Ok(ToolResult::FileList(out))
    }

    pub(super) fn read_file(&self, args: ReadFileArgs) -> Result<ToolResult> {
        // Check if this is a memory path (absolute, outside workspace)
        let abs_path = Path::new(&args.path);
        if abs_path.is_absolute() && self.is_memory_path(abs_path) {
            if abs_path.exists() && abs_path.is_file() {
                return self.do_read_file(&args.path, abs_path, args.max_bytes, args.line_range);
            }
            return Ok(ToolResult::Success(self.prompt(
                crate::prompts::keys::FILE_NOT_FOUND_MEMORY,
                &[("path", &args.path)],
            )));
        }

        let rel = sanitize_rel_path(&self.root, &args.path).unwrap_or_else(|_| args.path.clone());
        let filename = Path::new(&rel)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&rel);
        let path = self.root.join(&rel);

        if path.exists() && path.is_dir() {
            anyhow::bail!(
                "{}",
                self.prompt(
                    crate::prompts::keys::READ_IS_DIRECTORY,
                    &[("path", &rel)],
                )
            );
        }

        if path.exists() && path.is_file() {
            return self.do_read_file(&rel, &path, args.max_bytes, args.line_range);
        }

        // File not found, try smart search candidates.
        info!("File not found: {}. Attempting smart search...", rel);
        let candidates = self.smart_search_candidates(&rel, 10)?;
        if let Some(best_match) = candidates.first() {
            let full_path = self.root.join(best_match);
            let note = if candidates.len() > 1 {
                self.prompt(
                    crate::prompts::keys::SMART_SEARCH_REDIRECT_MULTI,
                    &[
                        ("original", &rel),
                        ("count", &candidates.len().to_string()),
                        ("found", best_match),
                        ("others", &candidates[1..].join(", ")),
                    ],
                )
            } else {
                self.prompt(
                    crate::prompts::keys::SMART_SEARCH_REDIRECT,
                    &[("original", &rel), ("found", best_match)],
                )
            };
            return self.do_read_file_with_note(
                best_match,
                &full_path,
                args.max_bytes,
                args.line_range,
                &note,
            );
        }

        // 3. Last resort: tell the model we searched everywhere
        Ok(ToolResult::Success(self.prompt(
            crate::prompts::keys::FILE_NOT_FOUND_EXHAUSTED,
            &[("rel", &rel), ("filename", filename)],
        )))
    }

    pub(super) fn smart_search_candidates(&self, query: &str, max_results: usize) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let limit = max_results.max(1);

        let filename = Path::new(query)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(query);

        // 1) Exact basename matches anywhere in the repo.
        if !filename.is_empty() {
            let glob_pattern = format!("**/{}", filename);
            if let Ok(ToolResult::FileList(matches)) = self.list_files(ListFilesArgs {
                globs: Some(vec![glob_pattern]),
                max_results: Some(limit),
            }) {
                for m in matches {
                    if seen.insert(m.clone()) {
                        out.push(m);
                        if out.len() >= limit {
                            return Ok(out);
                        }
                    }
                }
            }
        }

        // 2) Case-insensitive filename/path contains matches.
        let query_lower = query.to_lowercase();
        let filename_lower = filename.to_lowercase();
        let walker = WalkBuilder::new(&self.root)
            .standard_filters(true)
            .hidden(true)
            .build();

        for entry in walker {
            let entry = match entry {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            let rel = match to_rel_string(&self.root, path) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let rel_lower = rel.to_lowercase();
            let name_lower = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_lowercase();

            let name_match = !filename_lower.is_empty()
                && !name_lower.is_empty()
                && (name_lower == filename_lower
                    || name_lower.contains(&filename_lower)
                    || filename_lower.contains(&name_lower));
            let path_match = !query_lower.is_empty() && rel_lower.contains(&query_lower);
            if (name_match || path_match) && seen.insert(rel.clone()) {
                out.push(rel);
                if out.len() >= limit {
                    break;
                }
            }
        }

        Ok(out)
    }

    pub(super) fn do_read_file(
        &self,
        rel: &str,
        path: &Path,
        max_bytes: Option<usize>,
        line_range: Option<[usize; 2]>,
    ) -> Result<ToolResult> {
        let max = max_bytes.unwrap_or(64 * 1024);

        let (content, truncated) = if let Some([start, end]) = line_range {
            if start == 0 || end < start {
                anyhow::bail!(
                    "invalid line_range [{}, {}]; expected 1-based inclusive range with start <= end",
                    start,
                    end
                );
            }

            use std::io::BufRead;
            let file = fs::File::open(path)?;
            let reader = std::io::BufReader::new(file);
            let mut out = String::new();
            let mut truncated = false;

            for (idx, line_res) in reader.lines().enumerate() {
                let line_no = idx + 1;
                if line_no < start {
                    continue;
                }
                if line_no > end {
                    break;
                }
                let line = line_res?;
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&line);
                if out.len() > max {
                    out.truncate(max);
                    truncated = true;
                    break;
                }
            }

            (out, truncated)
        } else {
            let file = fs::File::open(path)?;
            let mut buf = Vec::new();
            use std::io::Read;
            file.take(max as u64 + 1).read_to_end(&mut buf)?;
            let truncated = buf.len() > max;
            if truncated {
                buf.truncate(max);
            }
            (String::from_utf8_lossy(&buf).to_string(), truncated)
        };

        Ok(ToolResult::FileContent {
            path: rel.to_string(),
            content,
            truncated,
        })
    }

    pub(super) fn do_read_file_with_note(
        &self,
        rel: &str,
        path: &Path,
        max_bytes: Option<usize>,
        line_range: Option<[usize; 2]>,
        note: &str,
    ) -> Result<ToolResult> {
        match self.do_read_file(rel, path, max_bytes, line_range)? {
            ToolResult::FileContent {
                path,
                content,
                truncated,
            } => Ok(ToolResult::FileContent {
                path: format!("{} ({})", path, note),
                content: format!("/* {} */\n\n{}", note, content),
                truncated,
            }),
            other => Ok(other),
        }
    }

    pub(super) fn capture_screenshot(&self, args: CaptureScreenshotArgs) -> Result<ToolResult> {
        use headless_chrome::Browser;

        // Validate URL to prevent SSRF: only allow http/https and block private IPs.
        let parsed_url: reqwest::Url = args
            .url
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;
        match parsed_url.scheme() {
            "http" | "https" => {}
            scheme => anyhow::bail!("Disallowed URL scheme: {}", scheme),
        }
        if let Some(host) = parsed_url.host_str() {
            let is_private = host == "localhost"
                || host == "127.0.0.1"
                || host == "::1"
                || host == "0.0.0.0"
                || host.starts_with("10.")
                || is_rfc1918_172(host)
                || host.starts_with("192.168.")
                || host.ends_with(".local");
            if is_private {
                anyhow::bail!(
                    "Disallowed URL host (private/internal address): {}",
                    host
                );
            }
        }

        let browser = Browser::default()?;
        let tab = browser.new_tab()?;

        tab.navigate_to(&args.url)?;
        tab.wait_until_navigated()?;

        // Wait a bit for dynamic content
        std::thread::sleep(Duration::from_millis(args.delay_ms.unwrap_or(1000)));

        let png_data = tab.capture_screenshot(
            headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png,
            None,
            None,
            true,
        )?;

        use base64::prelude::*;
        let b64 = BASE64_STANDARD.encode(png_data);

        Ok(ToolResult::Screenshot {
            url: args.url,
            base64: b64,
        })
    }
}
