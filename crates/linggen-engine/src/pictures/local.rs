//! Drawing on this Mac — one FLUX render per process, on the managed `pictures`
//! venv (see [`crate::runtime`]). The engine writes the runner script
//! beside the runtime, feeds one JSON request on stdin and reads one JSON
//! line back. Nothing stays resident: a render holds 7–14 GB of unified
//! memory and the model reloads from the cache in under a second, so the
//! process ends with the picture. Renders are serialized — two at once
//! would double that memory for no speed.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const RUNNER_SRC: &str = include_str!("../runtime_py/mlx_picture.py");

/// Generous: a 512² picture takes ~10–15 s on an M2 Max including load,
/// a reference edit at the wide size about a minute; a slower Mac gets
/// three times that before we give up.
const RENDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(240);

/// This Mac's sizes. Both fit a 16 GB Mac; larger sizes need 13 GB or
/// more mid-render and were measured out of the game's needs.
fn size(shape: super::Shape) -> (u32, u32) {
    match shape {
        super::Shape::Square => (512, 512),
        super::Shape::Landscape => (768, 512),
    }
}

/// Draw `ask` on this Mac and hand back the PNG; the render's file is a
/// scratch file under the runtime, gone once read.
pub async fn draw(ask: super::Ask) -> Result<super::Drawn> {
    let out = crate::runtime::runtime_dir()
        .join("pictures-tmp")
        .join(format!("{}.png", uuid::Uuid::new_v4()));
    let rendered = render(Request {
        prompt: ask.prompt,
        out: out.clone(),
        shape: ask.shape,
        seed: ask.seed,
        reference: ask.reference,
    })
    .await?;
    let png = tokio::fs::read(&out)
        .await
        .context("read the drawn picture");
    let _ = tokio::fs::remove_file(&out).await;
    Ok(super::Drawn {
        png: png?,
        source: super::Source::ThisMac,
        seconds: rendered.seconds,
        seed: Some(rendered.seed),
    })
}

pub struct Request {
    pub prompt: String,
    pub out: PathBuf,
    pub shape: super::Shape,
    pub seed: u64,
    pub reference: Option<PathBuf>,
}

pub struct Rendered {
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

    let (width, height) = size(req.shape);
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
        .env("HF_ENDPOINT", crate::hf_mirror::endpoint().await)
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
        seconds: v.get("seconds").and_then(|s| s.as_f64()).unwrap_or(0.0),
        seed: req.seed,
    })
}
