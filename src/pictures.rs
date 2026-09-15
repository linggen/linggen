//! Local pictures — one FLUX render per process, on the managed `pictures`
//! venv (see [`crate::runtime`]). The engine writes the runner script
//! beside the runtime, feeds one JSON request on stdin and reads one JSON
//! line back. Nothing stays resident: a render holds 7–14 GB of unified
//! memory and the model reloads from the cache in under a second, so the
//! process ends with the picture. Renders are serialized — two at once
//! would double that memory for no speed.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const RUNNER_SRC: &str = include_str!("runtime_py/mlx_picture.py");

/// Generous: a 512² picture takes ~10–15 s on an M2 Max including load,
/// a reference edit at the wide size about a minute; a slower Mac gets
/// three times that before we give up.
const RENDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(240);

/// The shapes a caller may ask for. Both fit a 16 GB Mac; larger sizes
/// need 13 GB or more mid-render and were measured out of the game's needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Square,
    Landscape,
}

impl Shape {
    pub fn parse(s: Option<&str>) -> Result<Self> {
        match s.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("square") => Ok(Shape::Square),
            Some("landscape") | Some("wide") => Ok(Shape::Landscape),
            Some(other) => bail!("shape must be square or landscape, not {other:?}"),
        }
    }

    pub fn size(self) -> (u32, u32) {
        match self {
            Shape::Square => (512, 512),
            Shape::Landscape => (768, 512),
        }
    }
}

pub struct Request {
    pub prompt: String,
    pub out: PathBuf,
    pub shape: Shape,
    pub seed: u64,
    pub reference: Option<PathBuf>,
}

pub struct Rendered {
    pub path: PathBuf,
    pub seconds: f64,
    pub seed: u64,
}

/// Render one picture. Fails plainly when the lane is not installed; the
/// caller decides whether to start the install.
pub async fn render(req: Request) -> Result<Rendered> {
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = ONE_AT_A_TIME.lock().await;

    if !crate::runtime::pictures_ready() {
        bail!("pictures are not installed on this machine yet");
    }
    let script = crate::runtime::runtime_dir().join("mlx_picture.py");
    tokio::fs::write(&script, RUNNER_SRC)
        .await
        .context("write picture runner")?;
    if let Some(parent) = req.out.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    let (width, height) = req.shape.size();
    let payload = serde_json::json!({
        "model": crate::runtime::PICTURE_MODEL,
        "prompt": req.prompt,
        "out": req.out,
        "width": width,
        "height": height,
        "seed": req.seed,
        "reference": req.reference,
    });

    let mut child = tokio::process::Command::new(crate::runtime::env_bin("pictures", "python3"))
        .arg(&script)
        .env("HF_HOME", crate::runtime::hf_home())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("spawn picture runner")?;

    let mut stdin = child.stdin.take().context("runner stdin")?;
    let stdout = child.stdout.take().context("runner stdout")?;
    let stderr = child.stderr.take().context("runner stderr")?;
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!("[pictures] {line}");
        }
    });
    stdin.write_all(payload.to_string().as_bytes()).await?;
    drop(stdin);

    let mut reply = String::new();
    let mut stdout = BufReader::new(stdout);
    let read = stdout.read_line(&mut reply);
    match tokio::time::timeout(RENDER_TIMEOUT, read).await {
        Ok(Ok(0)) => bail!("picture runner ended without a reply"),
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(e).context("read picture runner"),
        Err(_) => bail!("picture took longer than {}s", RENDER_TIMEOUT.as_secs()),
    }
    let _ = child.wait().await;

    let v: serde_json::Value =
        serde_json::from_str(reply.trim()).context("picture runner reply")?;
    if v.get("ok").and_then(|b| b.as_bool()) != Some(true) {
        let err = v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown error");
        bail!("{err}");
    }
    Ok(Rendered {
        path: req.out,
        seconds: v.get("seconds").and_then(|s| s.as_f64()).unwrap_or(0.0),
        seed: req.seed,
    })
}

/// A file stem safe for a URL and a folder: ASCII letters, digits, `-`,
/// `_`; anything else becomes `-`; empty falls back to `picture`.
pub fn safe_stem(name: &str) -> String {
    let mut stem = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            stem.push(c);
        } else if !stem.ends_with('-') {
            stem.push('-');
        }
    }
    let stem = stem.trim_matches('-').to_string();
    if stem.is_empty() {
        "picture".to_string()
    } else {
        stem.chars().take(64).collect()
    }
}

/// Resolve a reference picture the caller named: relative to `skill_dir`,
/// and it must stay inside it — the tool never reads outside the skill.
pub fn reference_inside(skill_dir: &Path, reference: &str) -> Result<PathBuf> {
    let candidate = skill_dir.join(reference.trim_start_matches('/'));
    let real = candidate
        .canonicalize()
        .with_context(|| format!("reference picture not found: {reference}"))?;
    let root = skill_dir.canonicalize().context("skill dir")?;
    if !real.starts_with(&root) {
        bail!("reference must be a file inside the skill's folder");
    }
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_parse_and_size() {
        assert_eq!(Shape::parse(None).unwrap(), Shape::Square);
        assert_eq!(Shape::parse(Some("wide")).unwrap().size(), (768, 512));
        assert_eq!(Shape::parse(Some("Landscape")).unwrap(), Shape::Landscape);
        assert!(Shape::parse(Some("huge")).is_err());
    }

    #[test]
    fn stems_are_url_safe() {
        assert_eq!(safe_stem("夫諸 1597 / plate"), "1597-plate");
        assert_eq!(safe_stem("  "), "picture");
        assert_eq!(safe_stem("iron_sword-2"), "iron_sword-2");
    }

    #[test]
    fn reference_stays_inside_the_skill() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("plate.jpg"), b"x").unwrap();
        assert!(reference_inside(dir.path(), "plate.jpg").is_ok());
        assert!(reference_inside(dir.path(), "../plate.jpg").is_err());
        assert!(reference_inside(dir.path(), "missing.jpg").is_err());
    }
}
