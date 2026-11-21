//! Whisper on the Mac's GPU, heard through the voice sidecar
//! ([`super::mlx_sidecar`], the process her voice already runs in).
//!
//! The samples go to the sidecar on its stdin, as one JSON line holding them
//! in base64, and come back as words. Nothing is written to disk and nothing
//! goes to the network: the sidecar hands the samples to the model as an
//! array and reads the model from the local cache by path
//! (`mlx_voice.py`; `the_sidecar_script_keeps_what_it_hears_to_itself`).
//! The model is named once, `runtime::STT_MODEL`; the machine it needs is the
//! voice lane's. Where hearing cannot happen the answer is a reason in words
//! ([`Unavailable`]): this machine's, or that the model is not here yet or
//! is being fetched.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;

use super::mlx_sidecar::VoiceSidecar;
use super::stt::{Heard, SttProvider, Unavailable, RATE};

/// The least a request waits: the sidecar may be loading the model, once.
const BASE_WAIT_SECS: u64 = 30;

pub struct MlxSttProvider {
    sidecar: Arc<VoiceSidecar>,
}

impl MlxSttProvider {
    pub fn new(sidecar: Arc<VoiceSidecar>) -> Self {
        Self { sidecar }
    }

    /// What this machine can do now, as the one verdict: the voice lane's
    /// gate, then the hearing model on disk, then whether it is on its way.
    fn verdict(lane: Result<(), String>, ready: bool, fetching: bool) -> Result<(), Unavailable> {
        lane.map_err(|reason| Unavailable::because(format!("Hearing {reason}.")))?;
        match (ready, fetching) {
            (true, _) => Ok(()),
            (false, true) => Err(Unavailable::fetching("Fetching the hearing model (1.6 GB). Try again when it is done.")),
            (false, false) => Err(Unavailable::because("The hearing model (1.6 GB) is not on this Mac yet.")),
        }
    }

    /// The words in `samples`, from the sidecar.
    async fn listen(&self, samples: &[i16]) -> anyhow::Result<Heard> {
        let pcm: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let request = serde_json::json!({
            "op": "hear",
            "pcm_b64": base64::engine::general_purpose::STANDARD.encode(pcm),
        });
        let wait = Duration::from_secs(BASE_WAIT_SECS + samples.len() as u64 / RATE as u64);
        let reply = self.sidecar.ask(&request, wait).await?;
        let text = reply
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("reply missing text"))?;
        let language = reply.get("language").and_then(|v| v.as_str()).unwrap_or("");
        Ok(Heard { text: text.to_string(), language: language.to_string() })
    }
}

#[async_trait]
impl SttProvider for MlxSttProvider {
    fn can_hear(&self) -> Result<(), Unavailable> {
        let lane = crate::runtime::lane_verdict(&crate::runtime::VOICE_LANE);
        Self::verdict(lane, crate::runtime::hearing_ready(), crate::runtime::hearing_fetching())
    }

    /// The same runtime path and progress as her voice's model, only on a
    /// machine whose voice lane is open and which lacks the model.
    fn begin_fetch(&self) {
        let lane_open = crate::runtime::lane_verdict(&crate::runtime::VOICE_LANE).is_ok();
        if lane_open && !crate::runtime::hearing_ready() {
            crate::runtime::ensure_hearing_in_background();
        }
    }

    async fn hear(&self, samples: &[i16]) -> Result<Heard, Unavailable> {
        self.listen(samples).await.map_err(|e| {
            tracing::warn!("[stt] {e:#}");
            Unavailable::because("Hearing stopped for a moment. Try again.")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sidecar that answers a `hear` request with words and anything else
    /// with an error, writing nothing anywhere.
    fn fake_sidecar(dir: &std::path::Path) -> Arc<VoiceSidecar> {
        let script = r#"echo '{"ready":true}'
while read line; do
  case "$line" in
    *'"op":"hear"'*) echo '{"ok":true,"text":"今天有什么安排 and the budget","language":"zh"}';;
    *) echo '{"ok":false,"error":"no op"}';;
  esac
done
"#;
        let path = dir.join("fake-voice.sh");
        std::fs::write(&path, script).unwrap();
        VoiceSidecar::with_program("/bin/sh".into(), vec![path.display().to_string()])
    }

    /// Every file under `dir` changed since `since` that holds `needle`.
    fn files_holding(dir: &std::path::Path, needle: &[u8], since: std::time::SystemTime, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let (path, Ok(meta)) = (entry.path(), entry.metadata()) else { continue };
            if meta.is_dir() {
                files_holding(&path, needle, since, out);
            } else if meta.len() < 16 << 20 && meta.modified().is_ok_and(|t| t >= since) {
                let bytes = std::fs::read(&path).unwrap_or_default();
                if bytes.windows(needle.len()).any(|w| w == needle) {
                    out.push(path.display().to_string());
                }
            }
        }
    }

    /// Every state a machine can be in has one verdict, and a no is always in
    /// words for a person, short enough to read at a glance.
    #[test]
    fn the_verdict_names_each_state_a_machine_can_be_in() {
        let verdict = MlxSttProvider::verdict;
        assert_eq!(verdict(Ok(()), true, false), Ok(()));
        assert_eq!(verdict(Ok(()), true, true), Ok(()), "the model arrived");
        let lane_closed = verdict(Err("needs an Apple Silicon Mac".into()), false, false).unwrap_err();
        assert_eq!(lane_closed, Unavailable::because("Hearing needs an Apple Silicon Mac."));
        let lane_closed_fetching = verdict(Err("needs a Mac".into()), false, true).unwrap_err();
        assert!(!lane_closed_fetching.fetching, "a machine that cannot hear is not fetching");
        let missing = verdict(Ok(()), false, false).unwrap_err();
        let fetching = verdict(Ok(()), false, true).unwrap_err();
        assert!(!missing.fetching && fetching.fetching);
        for no in [lane_closed, missing, fetching] {
            assert!(no.reason.split_whitespace().count() <= 20, "{}", no.reason);
        }
    }

    #[tokio::test]
    async fn the_providers_samples_reach_the_sidecar_and_its_words_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let hearing = MlxSttProvider::new(fake_sidecar(dir.path()));
        let heard = hearing.listen(&[100i16; 16_000]).await.unwrap();
        assert_eq!(heard.text, "今天有什么安排 and the budget");
        assert_eq!(heard.language, "zh");
    }

    /// After a call, no file anywhere this process writes holds the sound —
    /// not as samples, not as the base64 the sidecar is sent.
    #[tokio::test]
    async fn no_sound_is_left_behind_on_disk() {
        let started = std::time::SystemTime::now() - Duration::from_secs(1);
        let dir = tempfile::tempdir().unwrap();
        let hearing = MlxSttProvider::new(fake_sidecar(dir.path()));
        // A pattern no other file holds.
        let samples: Vec<i16> = (0..16_000).map(|i| 0x1234 + (i % 7) as i16 * 0x0111).collect();
        hearing.listen(&samples).await.unwrap();

        let raw: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
        let mut found = Vec::new();
        for needle in [&raw[..64], &b64.as_bytes()[..64]] {
            for place in [std::env::temp_dir(), crate::paths::linggen_home().to_path_buf(), dir.path().to_path_buf()] {
                files_holding(&place, needle, started, &mut found);
            }
        }
        assert!(found.is_empty(), "the sound was written to: {found:?}");
    }

    /// The sidecar's script, the half of the promise Rust cannot see: it
    /// opens no file and no connection for what it hears, and loads the model
    /// by local path.
    #[test]
    fn the_sidecar_script_keeps_what_it_hears_to_itself() {
        let script = include_str!("../../runtime_py/mlx_voice.py");
        let code: String = script
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        for word in [
            "tempfile", "NamedTemporary", "socket", "urllib", "requests", "httpx", "aiohttp", "http.client",
            "soundfile", "write_bytes", "hf_hub_download", "from_pretrained",
        ] {
            assert!(!code.contains(word), "mlx_voice.py names `{word}`");
        }
        // The function that hears opens no file at all.
        let hear = &code[code.find("def hear(").unwrap()..code.find("def main(").unwrap()];
        for word in ["open(", "write(", "save(", "dump("] {
            assert!(!hear.contains(word), "hear() calls `{word}`");
        }
        // Every hub call in it stays on this machine.
        for call in code.match_indices("snapshot_download(") {
            let rest = &code[call.0..];
            let line = rest.lines().next().unwrap_or("");
            assert!(line.contains("local_files_only=True"), "a hub call that may go online: {line}");
        }
        // The model it loads for hearing is the one the runtime names, by path.
        assert!(code.contains("snapshot_download(state[\"stt_id\"], local_files_only=True)"));
        assert!(code.contains("load_stt(Path(path))"));
    }

    /// The model is named once, beside the voice model, and fetched in the
    /// voice lane — no third lane.
    #[test]
    fn hearing_is_named_in_the_runtime_and_nowhere_else() {
        assert!(crate::runtime::STT_MODEL.contains("whisper-large-v3-turbo"));
        for (rel, code) in crate::server::lock_scan::production_code(&["src", "crates/linggen-engine/src"]) {
            if rel.ends_with("runtime.rs") {
                continue;
            }
            assert!(!code.contains("whisper"), "{rel} names a model: models are named in runtime.rs alone");
        }
    }
}
