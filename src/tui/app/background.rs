use super::App;
use anyhow::Result;

impl App {
    /// Fetch skills and agents from the server in the background for autocomplete.
    pub fn fetch_autocomplete_data(&self) {
        let client = self.client.clone();
        let project_root = self.project_root.clone();
        let skills_slot = self.skills_slot.clone();
        let agents_slot = self.agents_slot.clone();
        let models_slot = self.models_slot.clone();
        tokio::spawn(async move {
            // Fetch skills
            match client.fetch_skills().await {
                Ok(skills) => {
                    let parsed: Vec<(String, String)> = skills
                        .iter()
                        .filter_map(|s| {
                            let name = s.get("name").and_then(|v| v.as_str())?;
                            let desc = s
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            Some((name.to_string(), desc.to_string()))
                        })
                        .collect();
                    *skills_slot.lock().unwrap() = Some(parsed);
                }
                Err(e) => {
                    tracing::debug!("Failed to fetch skills for autocomplete: {}", e);
                }
            }
            // Fetch agents
            match client.fetch_agents(&project_root).await {
                Ok(agents) => {
                    let parsed: Vec<(String, String)> = agents
                        .iter()
                        .filter_map(|a| {
                            let name = a.get("name").and_then(|v| v.as_str())?;
                            let desc = a
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            Some((name.to_string(), desc.to_string()))
                        })
                        .collect();
                    *agents_slot.lock().unwrap() = Some(parsed);
                }
                Err(e) => {
                    tracing::debug!("Failed to fetch agents for autocomplete: {}", e);
                }
            }
            // Fetch models
            match client.fetch_models().await {
                Ok(models) => {
                    let parsed: Vec<(String, String)> = models
                        .iter()
                        .filter_map(|m| {
                            let id = m.get("id").and_then(|v| v.as_str())?;
                            let provider = m
                                .get("provider")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            let model = m
                                .get("model")
                                .and_then(|v| v.as_str())
                                .unwrap_or(id);
                            Some((id.to_string(), format!("{}: {}", provider, model)))
                        })
                        .collect();
                    *models_slot.lock().unwrap() = Some(parsed);
                }
                Err(e) => {
                    tracing::debug!("Failed to fetch models for autocomplete: {}", e);
                }
            }
        });
    }

    /// Check shared slots and populate caches if data has arrived.
    pub fn check_autocomplete_slots(&mut self) {
        if let Some(skills) = self.skills_slot.lock().unwrap().take() {
            self.cached_skills = skills;
        }
        if let Some(agents) = self.agents_slot.lock().unwrap().take() {
            self.cached_agents = agents;
        }
        if let Some(models) = self.models_slot.lock().unwrap().take() {
            self.cached_models = models;
        }
    }

    /// Trigger a full state resync from the server via REST APIs.
    /// Spawns a fire-and-forget background task; errors are logged.
    pub(super) fn trigger_resync(&self) {
        let client = self.client.clone();
        let project_root = self.project_root.clone();
        let session_id = self.session_id.clone();
        tokio::spawn(async move {
            if let Err(e) = client
                .fetch_workspace_state(&project_root, session_id.as_deref())
                .await
            {
                tracing::debug!("Resync workspace state failed: {}", e);
            }
            if let Err(e) = client
                .fetch_agent_runs(&project_root, session_id.as_deref())
                .await
            {
                tracing::debug!("Resync agent runs failed: {}", e);
            }
        });
    }

    /// Grab a PNG image from the system clipboard and return base64.
    /// macOS: uses osascript, Linux: tries wl-paste then xclip.
    pub(super) fn grab_clipboard_image() -> Result<String> {
        use base64::Engine;
        let tmp_path = std::env::temp_dir().join("linggen_clipboard_img.png");

        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("osascript")
                .arg("-e")
                .arg(format!(
                    "set imgData to the clipboard as «class PNGf»\n\
                     set fp to open for access POSIX file \"{}\" with write permission\n\
                     write imgData to fp\n\
                     close access fp",
                    tmp_path.display()
                ))
                .output()
                .map_err(|e| anyhow::anyhow!("osascript failed: {}", e))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("clipboard has no image ({})", stderr.trim());
            }
        }

        #[cfg(target_os = "linux")]
        {
            let wl_ok = std::process::Command::new("wl-paste")
                .args(["--type", "image/png"])
                .stdout(std::fs::File::create(&tmp_path)?)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !wl_ok {
                let xclip_ok = std::process::Command::new("xclip")
                    .args(["-selection", "clipboard", "-target", "image/png", "-o"])
                    .stdout(std::fs::File::create(&tmp_path)?)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if !xclip_ok {
                    anyhow::bail!("no image in clipboard (tried wl-paste and xclip)");
                }
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            anyhow::bail!("clipboard image paste not supported on this platform");
        }

        if !tmp_path.exists() {
            anyhow::bail!("clipboard image file was not created");
        }
        let data = std::fs::read(&tmp_path)?;
        let _ = std::fs::remove_file(&tmp_path);
        if data.is_empty() {
            anyhow::bail!("clipboard image is empty");
        }
        Ok(base64::engine::general_purpose::STANDARD.encode(&data))
    }

    /// Copy the last agent message to the system clipboard.
    pub(super) fn copy_last_agent_message(&mut self) {
        let last_text = self.blocks.iter().rev().find_map(|block| {
            if let super::super::display::DisplayBlock::AgentMessage { text, .. } = block {
                Some(text.clone())
            } else {
                None
            }
        });
        match last_text {
            Some(text) => {
                match Self::copy_to_clipboard(&text) {
                    Ok(()) => self.push_system("Copied last agent message to clipboard."),
                    Err(e) => self.push_system(&format!("Failed to copy: {e}")),
                }
            }
            None => self.push_system("No agent message to copy."),
        }
    }

    /// Copy text to the system clipboard.
    fn copy_to_clipboard(text: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            use std::io::Write;
            let mut child = std::process::Command::new("pbcopy")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| anyhow::anyhow!("pbcopy failed: {}", e))?;
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(text.as_bytes())?;
            child.wait()?;
            return Ok(());
        }

        #[cfg(target_os = "linux")]
        {
            use std::io::Write;
            // Try wl-copy first (Wayland), then xclip (X11)
            let wl = std::process::Command::new("wl-copy")
                .stdin(std::process::Stdio::piped())
                .spawn();
            if let Ok(mut child) = wl {
                child
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(text.as_bytes())?;
                child.wait()?;
                return Ok(());
            }
            let mut child = std::process::Command::new("xclip")
                .args(["-selection", "clipboard"])
                .stdin(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| anyhow::anyhow!("xclip failed: {}", e))?;
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(text.as_bytes())?;
            child.wait()?;
            return Ok(());
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            anyhow::bail!("clipboard copy not supported on this platform");
        }
    }

    /// Try pasting a clipboard image and push a system status message.
    pub(super) fn paste_clipboard_image(&mut self) {
        match Self::grab_clipboard_image() {
            Ok(base64) => {
                self.pending_images.push(base64);
                let count = self.pending_images.len();
                self.push_system(&format!("Image pasted from clipboard ({count} pending)"));
            }
            Err(e) => {
                self.push_system(&format!("No image in clipboard: {e}"));
            }
        }
    }

    /// Handle /image and /paste commands.
    pub(super) fn handle_image_command(&mut self, line: &str) {
        if line == "/paste" {
            self.paste_clipboard_image();
            return;
        }
        if line == "/image" {
            self.push_system("Usage: /image <file_path>  — attach an image file");
            self.push_system("       Ctrl+V             — paste image from clipboard");
            self.push_system(&format!("  {} image(s) pending", self.pending_images.len()));
            return;
        }
        if line == "/image clear" {
            self.pending_images.clear();
            self.push_system("Cleared all pending images.");
            return;
        }
        // /image <path>
        let path = line.strip_prefix("/image ").unwrap_or("").trim();
        if path.is_empty() {
            self.push_system("Usage: /image <file_path>");
            return;
        }
        match Self::load_image_file(path) {
            Ok(base64) => {
                self.pending_images.push(base64);
                let count = self.pending_images.len();
                self.push_system(&format!("Image attached: {path} ({count} pending)"));
            }
            Err(e) => {
                self.push_system(&format!("Failed to load image: {e}"));
            }
        }
    }

    /// Load an image file from disk and return its base64-encoded content.
    pub(super) fn load_image_file(path: &str) -> Result<String> {
        use base64::Engine;
        let expanded = if path.starts_with('~') {
            if let Some(home) = dirs::home_dir() {
                home.join(path.strip_prefix("~/").unwrap_or(path))
            } else {
                std::path::PathBuf::from(path)
            }
        } else {
            std::path::PathBuf::from(path)
        };
        let data = std::fs::read(&expanded)
            .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", expanded.display(), e))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(&data))
    }
}
