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
//! ([`Unavailable`]): this machine's, or that the model is not here yet, is
//! being fetched, or is being got ready.
//!
//! Hearing has two steps before the first clip, each its own attempt and
//! neither inside a hear's short wait: the fetch (`runtime`), then the load
//! (`MlxSttProvider::loader`), started when the fetch ends, at a press that
//! finds the model on disk but not loaded, and — on a Mac that already holds
//! the model — at start, after her voice (`warm_if_fetched`).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;

use super::mlx_sidecar::VoiceSidecar;
use super::stt::{Heard, SttProvider, Unavailable, RATE};
use crate::runtime::{Attempt, AttemptState};

/// The least a hear waits for the model's reply once it has the slot. The
/// model is loaded and warmed (`mlx_voice.py`'s `load_hearing` runs one silent
/// clip through it), so this is slack, not the first clip's kernel compile.
const BASE_WAIT_SECS: u64 = 30;

/// What a segment may report and still be speech. The numbers are the model
/// library's own defaults for telling silence and failed decodes from words
/// (mlx-audio 0.5.0, the speech-to-text model's `generate`: `no_speech_threshold` 0.6,
/// `logprob_threshold` -1.0, `compression_ratio_threshold` 2.4), which the
/// library applies to a clip as a whole and the sidecar hands back per
/// segment; judged here, beside the quiet-clip check in `stt.rs`.
const NO_SPEECH_PROB_MAX: f64 = 0.6;
const AVG_LOGPROB_MIN: f64 = -1.0;
const COMPRESSION_RATIO_MAX: f64 = 2.4;

/// The line while the model is being got ready — said at a press, never kept.
const PREPARING: &str = "Getting ready to listen…";

/// How the clip's language shapes what is heard: the language the clip is in
/// when it names one the model hears, else the model finds it by itself; and
/// for Simplified Chinese a prompt that keeps the script Simplified (the model
/// otherwise picks a script at random). Which language the clip is in is
/// `stt::language_to_hear`'s decision, never made here.
#[derive(Debug, PartialEq)]
struct Hint {
    language: Option<&'static str>,
    prompt: Option<&'static str>,
}

/// The languages Linggen names (`language::display_name`) that the model hears
/// under the same two-letter code.
const HEARD_LANGUAGES: [&str; 10] = ["zh", "en", "ja", "ko", "fr", "de", "es", "pt", "it", "ru"];
const SIMPLIFIED_PROMPT: &str = "以下是普通话的句子。";

/// The hint for `language`: a BCP-47 tag, or none.
fn hint_of(language: Option<&str>) -> Hint {
    let tag = language.unwrap_or("").to_ascii_lowercase().replace('_', "-");
    let mut parts = tag.split('-');
    let Some(language) = parts.next().and_then(|code| HEARD_LANGUAGES.iter().find(|l| **l == code)) else {
        return Hint { language: None, prompt: None };
    };
    // Traditional by script or by region: no Simplified prompt.
    let traditional = parts.any(|p| matches!(p, "hant" | "tw" | "hk" | "mo"));
    let prompt = (*language == "zh" && !traditional).then_some(SIMPLIFIED_PROMPT);
    Hint { language: Some(language), prompt }
}

/// The request the sidecar hears a clip from.
fn hear_request(samples: &[i16], hint: &Hint) -> serde_json::Value {
    let pcm: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    serde_json::json!({
        "op": "hear",
        "pcm_b64": base64::engine::general_purpose::STANDARD.encode(pcm),
        "language": hint.language,
        "prompt": hint.prompt,
    })
}

/// The words of a reply: the segments that are speech, joined. A reply with no
/// segment list holds its text as it is.
fn words_of(reply: &serde_json::Value) -> Option<String> {
    let Some(segments) = reply.get("segments").and_then(|v| v.as_array()) else {
        return reply.get("text").and_then(|v| v.as_str()).map(|t| t.trim().to_string());
    };
    let number = |seg: &serde_json::Value, key: &str, if_absent: f64| seg.get(key).and_then(|v| v.as_f64()).unwrap_or(if_absent);
    let words: String = segments
        .iter()
        .filter(|seg| {
            number(seg, "no_speech_prob", 0.0) <= NO_SPEECH_PROB_MAX
                && number(seg, "avg_logprob", 0.0) >= AVG_LOGPROB_MIN
                && number(seg, "compression_ratio", 0.0) <= COMPRESSION_RATIO_MAX
        })
        .filter_map(|seg| seg.get("text").and_then(|v| v.as_str()))
        .collect();
    Some(words.trim().to_string())
}

/// Everything the one verdict reads.
#[derive(Clone)]
struct Standing {
    /// The voice lane's gate: Err(why this machine cannot).
    lane: Result<(), String>,
    /// The venv and the voice model are on disk (hearing rides on them).
    voice_ready: bool,
    /// Those and the hearing model.
    model_on_disk: bool,
    fetch: AttemptState,
    loaded: bool,
    loading: AttemptState,
}

/// What a press does next about what hearing waits on.
#[derive(Debug, PartialEq)]
enum Step {
    Fetch,
    Load,
    Nothing,
}

/// Fetch what is missing, else load what is not loaded — never on a machine
/// whose voice lane is closed.
fn next_step(lane_open: bool, model_on_disk: bool, loaded: bool) -> Step {
    match (lane_open, model_on_disk, loaded) {
        (false, _, _) | (true, true, true) => Step::Nothing,
        (true, false, _) => Step::Fetch,
        (true, true, false) => Step::Load,
    }
}

/// The hearing model's load: its own attempt, in a task of its own.
#[derive(Clone)]
struct Loader {
    sidecar: Arc<VoiceSidecar>,
    attempt: Attempt,
}

impl Loader {
    /// Start loading unless a load runs. The task is spawned, so a request
    /// that is dropped never abandons a load half way.
    fn start(&self) {
        let Some(run) = self.attempt.begin() else { return };
        let sidecar = self.sidecar.clone();
        tokio::spawn(crate::runtime::run_attempt(run, async move { sidecar.load_hearing().await }, || ()));
    }
}

pub struct MlxSttProvider {
    sidecar: Arc<VoiceSidecar>,
    loader: Loader,
}

impl MlxSttProvider {
    pub fn new(sidecar: Arc<VoiceSidecar>) -> Self {
        Self { sidecar: sidecar.clone(), loader: Loader { sidecar, attempt: Attempt::default() } }
    }

    /// What this machine can do now, as the one verdict: the voice lane's
    /// gate, then the hearing model on disk (or on its way, or failed), then
    /// loaded (or being got ready).
    fn verdict(s: Standing) -> Result<(), Unavailable> {
        s.lane.map_err(|reason| Unavailable::because(format!("Hearing {reason}.")))?;
        if !s.model_on_disk {
            return Err(match (s.fetch, s.voice_ready) {
                (AttemptState::Running { retry }, voice_ready) => Unavailable::fetching(fetch_line(retry, voice_ready)),
                (AttemptState::Failed, _) => Unavailable::because("Couldn't fetch the hearing model. Hold again to try again."),
                (AttemptState::Idle, _) => Unavailable::because("The hearing model (1.6 GB) is not on this Mac yet."),
            });
        }
        if s.loaded {
            return Ok(());
        }
        Err(match s.loading {
            AttemptState::Running { retry: false } => Unavailable::fetching(PREPARING),
            AttemptState::Running { retry: true } => Unavailable::fetching("The last try failed; getting ready to listen again…"),
            AttemptState::Failed => Unavailable::because("Couldn't get ready to listen. Hold again to try again."),
            AttemptState::Idle => Unavailable::because(PREPARING),
        })
    }

    /// The words in `samples`, from the sidecar.
    async fn listen(&self, samples: &[i16], language: &str) -> anyhow::Result<Heard> {
        let request = hear_request(samples, &hint_of(Some(language)));
        let wait = Duration::from_secs(BASE_WAIT_SECS + samples.len() as u64 / RATE as u64 / 2);
        let reply = self.sidecar.ask_to_hear(&request, wait).await?;
        let text = words_of(&reply).ok_or_else(|| anyhow::anyhow!("reply missing text"))?;
        let language = reply.get("language").and_then(|v| v.as_str()).unwrap_or("");
        Ok(Heard { text, language: language.to_string() })
    }
}

/// What is being fetched, in words: only the hearing model, or the voice
/// environment under it too (a Mac that has not built her voice yet).
fn fetch_line(retry: bool, voice_ready: bool) -> String {
    let what = match voice_ready {
        true => "the hearing model (1.6 GB)",
        false => "her voice and hearing (about 4 GB)",
    };
    match retry {
        true => format!("The last try failed; fetching {what} again. Hold again when it is done."),
        false => format!("Fetching {what}. Hold again when it is done."),
    }
}

#[async_trait]
impl SttProvider for MlxSttProvider {
    fn can_hear(&self) -> Result<(), Unavailable> {
        Self::verdict(Standing {
            lane: crate::runtime::lane_verdict(&crate::runtime::VOICE_LANE),
            voice_ready: crate::runtime::voice_ready(),
            model_on_disk: crate::runtime::hearing_ready(),
            fetch: crate::runtime::hearing_fetch(),
            loaded: self.sidecar.hearing_loaded(),
            loading: self.loader.attempt.state(),
        })
    }

    /// The next thing hearing waits on, begun: the model's fetch (one attempt;
    /// loading follows when it ends), or its load. A press that finds both
    /// done starts nothing.
    fn begin_fetch(&self) {
        let lane_open = crate::runtime::lane_verdict(&crate::runtime::VOICE_LANE).is_ok();
        match next_step(lane_open, crate::runtime::hearing_ready(), self.sidecar.hearing_loaded()) {
            Step::Fetch => {
                let loader = self.loader.clone();
                crate::runtime::ensure_hearing_in_background(move || loader.start());
            }
            Step::Load => self.loader.start(),
            Step::Nothing => {}
        }
    }

    /// At a start: a Mac that already holds the hearing model loads it, so
    /// the first press does not wait. Only the load step — never a fetch (a
    /// Mac that never fetched it does nothing until a press asks).
    fn warm_if_fetched(&self) {
        let lane_open = crate::runtime::lane_verdict(&crate::runtime::VOICE_LANE).is_ok();
        if let Step::Load = next_step(lane_open, crate::runtime::hearing_ready(), self.sidecar.hearing_loaded()) {
            self.loader.start();
        }
    }

    async fn hear(&self, samples: &[i16], language: &str) -> Result<Heard, Unavailable> {
        // Never inside a hear's wait: a load is its own step.
        if !self.sidecar.hearing_loaded() {
            self.loader.start();
            return Err(Unavailable::fetching(PREPARING));
        }
        self.listen(samples, language).await.map_err(|e| {
            tracing::warn!("[stt] {e:#}");
            Unavailable::because("Hearing stopped for a moment. Try again.")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sidecar that answers a `load` and a `hear` request and anything else
    /// with an error, writing nothing anywhere. Its hear holds one stock
    /// phrase the model would invent from a noisy room, with the numbers that
    /// say so.
    fn fake_sidecar(dir: &std::path::Path) -> Arc<VoiceSidecar> {
        let script = r#"echo '{"ready":true}'
while read line; do
  case "$line" in
    *'"op":"load"'*) echo '{"ok":true}';;
    *'"op":"hear"'*) echo '{"ok":true,"text":"今天有什么安排 and the budget Thank you for watching.","language":"zh","segments":[{"text":"今天有什么安排 and the budget","no_speech_prob":0.02,"avg_logprob":-0.3,"compression_ratio":1.1},{"text":" Thank you for watching.","no_speech_prob":0.93,"avg_logprob":-0.7,"compression_ratio":1.0}]}';;
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

    fn standing() -> Standing {
        Standing {
            lane: Ok(()),
            voice_ready: true,
            model_on_disk: true,
            fetch: AttemptState::Idle,
            loaded: true,
            loading: AttemptState::Idle,
        }
    }

    /// Every state a machine can be in has one verdict, and a no is always in
    /// words for a person, short enough to read at a glance.
    #[test]
    fn the_verdict_names_each_state_a_machine_can_be_in() {
        let verdict = MlxSttProvider::verdict;
        let running = AttemptState::Running { retry: false };
        let retrying = AttemptState::Running { retry: true };
        assert_eq!(verdict(standing()), Ok(()));
        assert_eq!(verdict(Standing { fetch: running, ..standing() }), Ok(()), "the model arrived and is loaded");
        let lane_closed = verdict(Standing { lane: Err("needs an Apple Silicon Mac".into()), model_on_disk: false, ..standing() }).unwrap_err();
        assert_eq!(lane_closed, Unavailable::because("Hearing needs an Apple Silicon Mac."));
        let closed_fetching = verdict(Standing { lane: Err("needs a Mac".into()), model_on_disk: false, fetch: running, ..standing() }).unwrap_err();
        assert!(!closed_fetching.fetching, "a machine that cannot hear is not fetching");

        let missing = Standing { model_on_disk: false, loaded: false, ..standing() };
        let not_here = verdict(Standing { ..missing.clone() }).unwrap_err();
        assert!(!not_here.fetching && not_here.reason.contains("not on this Mac yet"));
        let fetching = verdict(Standing { fetch: running, ..missing.clone() }).unwrap_err();
        assert!(fetching.fetching && fetching.reason.contains("hearing model (1.6 GB)"), "{}", fetching.reason);
        // The voice environment is under it too: the line says what is really fetched.
        let building = verdict(Standing { fetch: running, voice_ready: false, ..missing.clone() }).unwrap_err();
        assert!(building.fetching && building.reason.contains("her voice and hearing"), "{}", building.reason);
        // A fetch that failed is not "fetching"; its retry says it is one.
        let failed = verdict(Standing { fetch: AttemptState::Failed, ..missing.clone() }).unwrap_err();
        assert!(!failed.fetching && failed.reason.contains("Couldn't fetch"), "{}", failed.reason);
        let retry = verdict(Standing { fetch: retrying, ..missing.clone() }).unwrap_err();
        assert!(retry.fetching && retry.reason.contains("last try failed"), "{}", retry.reason);

        // On disk, not loaded: getting ready, then failed, then its retry.
        let unloaded = Standing { loaded: false, ..standing() };
        let idle = verdict(Standing { ..unloaded.clone() }).unwrap_err();
        assert_eq!(idle, Unavailable::because(PREPARING), "a press knocks to start the load");
        let loading = verdict(Standing { loading: running, ..unloaded.clone() }).unwrap_err();
        assert_eq!(loading, Unavailable::fetching(PREPARING));
        let load_failed = verdict(Standing { loading: AttemptState::Failed, ..unloaded.clone() }).unwrap_err();
        assert!(!load_failed.fetching && load_failed.reason.contains("Couldn't get ready"));
        let load_retry = verdict(Standing { loading: retrying, ..unloaded.clone() }).unwrap_err();
        assert!(load_retry.fetching && load_retry.reason.contains("last try failed"));

        for no in [lane_closed, not_here, fetching, building, failed, retry, idle, loading, load_failed, load_retry] {
            assert!(no.reason.split_whitespace().count() <= 20, "{}", no.reason);
        }
    }

    /// A press begins the next thing hearing waits on, and nothing when it waits on nothing.
    #[test]
    fn a_press_fetches_what_is_missing_then_loads_and_never_on_a_closed_lane() {
        assert_eq!(next_step(true, false, false), Step::Fetch);
        assert_eq!(next_step(true, true, false), Step::Load);
        assert_eq!(next_step(true, true, true), Step::Nothing);
        assert_eq!(next_step(false, false, false), Step::Nothing, "a Mac that cannot hear fetches nothing");
        assert_eq!(next_step(false, true, false), Step::Nothing);
    }

    /// The clip's language is the model's language when it names one the model
    /// hears, with a Simplified prompt for Chinese and never for a Traditional
    /// tag; a tag the model does not know is left to find it.
    #[test]
    fn the_clips_language_is_the_hint_and_simplified_chinese_keeps_its_script() {
        let hint = |setting| hint_of(setting);
        assert_eq!(hint(None), Hint { language: None, prompt: None });
        assert_eq!(hint(Some("")), Hint { language: None, prompt: None });
        assert_eq!(hint(Some("en")), Hint { language: Some("en"), prompt: None });
        assert_eq!(hint(Some("en-US")), Hint { language: Some("en"), prompt: None });
        for simplified in ["zh", "zh-Hans", "zh-CN", "zh_Hans_CN"] {
            assert_eq!(hint(Some(simplified)), Hint { language: Some("zh"), prompt: Some(SIMPLIFIED_PROMPT) }, "{simplified}");
        }
        for traditional in ["zh-Hant", "zh-TW", "zh-HK", "zh-Hant-TW"] {
            assert_eq!(hint(Some(traditional)), Hint { language: Some("zh"), prompt: None }, "{traditional}");
        }
        // Not a code the model is told: left to find it.
        assert_eq!(hint(Some("tlh")), Hint { language: None, prompt: None });
        assert_eq!(hint(Some("zhx")), Hint { language: None, prompt: None }, "a bare code, not a prefix");
        // The request carries both, absent as null.
        let request = hear_request(&[1, 2], &hint(Some("zh-Hans")));
        assert_eq!((request["op"].as_str(), request["language"].as_str(), request["prompt"].as_str()), (Some("hear"), Some("zh"), Some(SIMPLIFIED_PROMPT)));
        assert!(hear_request(&[1], &hint(None))["language"].is_null());
    }

    /// Rule 3: a segment the model reports as silence, or as a failed decode,
    /// is not words — whatever it says; the words that are, are kept.
    #[test]
    fn segments_the_model_reports_as_no_speech_are_not_words() {
        let reply = |segments: serde_json::Value| serde_json::json!({ "ok": true, "text": "x", "segments": segments });
        let seg = |text: &str, no_speech: f64, logprob: f64, ratio: f64| serde_json::json!(
            { "text": text, "no_speech_prob": no_speech, "avg_logprob": logprob, "compression_ratio": ratio });
        let said = reply(serde_json::json!([seg("What is on", 0.01, -0.2, 1.2), seg(" today?", 0.05, -0.4, 1.3)]));
        assert_eq!(words_of(&said).as_deref(), Some("What is on today?"));
        for (what, bad) in [
            ("no speech", seg(" Thank you for watching.", 0.9, -0.5, 1.0)),
            ("low confidence", seg(" 字幕", 0.1, -1.4, 1.0)),
            ("a repetition loop", seg(" ha ha ha ha", 0.1, -0.3, 3.5)),
        ] {
            assert_eq!(words_of(&reply(serde_json::json!([bad]))).as_deref(), Some(""), "{what}");
        }
        let mixed = reply(serde_json::json!([seg("Allow once", 0.02, -0.3, 1.0), seg(" Thank you.", 0.95, -0.6, 1.0)]));
        assert_eq!(words_of(&mixed).as_deref(), Some("Allow once"));
        // A reply with no segment list is its text; with neither, none.
        assert_eq!(words_of(&serde_json::json!({ "text": " hi " })).as_deref(), Some("hi"));
        assert_eq!(words_of(&serde_json::json!({})), None);
    }

    /// The wait for one hearing — the slot, then the model — fits the page's
    /// budget for it (`HEAR_BUDGET_MS` in shared/hold-to-talk.js), whatever the clip's length.
    #[test]
    fn the_waits_of_one_hearing_fit_the_pages_budget() {
        let page = include_str!("../../../shared/hold-to-talk.js");
        let at = page.find("export const HEAR_BUDGET_MS = ").expect("the page names its budget");
        let digits: String = page[at + "export const HEAR_BUDGET_MS = ".len()..].chars().take_while(|c| *c != ';').filter(|c| c.is_ascii_digit()).collect();
        let budget = Duration::from_millis(digits.parse().unwrap());
        let longest = Duration::from_secs(BASE_WAIT_SECS + (super::super::stt::MAX_SECS as u64) / 2);
        assert!(super::super::mlx_sidecar::HEAR_SLOT_WAIT + longest < budget, "{budget:?}");
    }

    #[tokio::test]
    async fn the_providers_samples_reach_the_sidecar_and_its_words_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let hearing = MlxSttProvider::new(fake_sidecar(dir.path()));
        let heard = hearing.listen(&[100i16; 16_000], "zh").await.unwrap();
        assert_eq!(heard.text, "今天有什么安排 and the budget", "the stock phrase the model reported as no speech is dropped");
        assert_eq!(heard.language, "zh");
    }

    /// Rule 2: a hear is never the load. Before the model is loaded a clip
    /// is not sent — the person is told she is getting ready — and the load
    /// is started in its own task; once it is loaded, the same clip is heard.
    #[tokio::test]
    async fn a_hear_before_the_load_is_not_sent_and_starts_the_load() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = fake_sidecar(dir.path());
        let hearing = MlxSttProvider::new(sidecar.clone());
        let refused = hearing.hear(&[100i16; 16_000], "zh").await.unwrap_err();
        assert_eq!(refused, Unavailable::fetching(PREPARING));
        for _ in 0..100 {
            if sidecar.hearing_loaded() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(sidecar.hearing_loaded(), "the load ran in its own task");
        let heard = hearing.hear(&[100i16; 16_000], "zh").await.unwrap();
        assert_eq!(heard.language, "zh");
    }

    /// A start loads only what is already here: a Mac without the hearing
    /// model on disk (a test's world has none) starts neither a fetch nor a load.
    #[tokio::test]
    async fn a_start_on_a_mac_that_never_fetched_hearing_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = fake_sidecar(dir.path());
        let hearing = MlxSttProvider::new(sidecar.clone());
        hearing.warm_if_fetched();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(hearing.loader.attempt.state(), AttemptState::Idle);
        assert!(!sidecar.hearing_loaded());
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
        hearing.listen(&samples, "zh").await.unwrap();

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
        // A hear is never the load, and prints nothing: the library is asked
        // for no output (`verbose=None`; `False` draws a progress bar, `True`
        // prints the words), and the load is a step of its own.
        assert!(hear.contains(r#"options = {"verbose": None}"#), "hear() must ask the library to print nothing");
        assert!(!hear.contains("snapshot_download") && !hear.contains("load_model"), "hear() loads nothing");
        assert!(hear.contains("raise NotLoaded()"));
        assert!(code.contains("def load_hearing("));
        // The load warms the model, so a hear's short wait never holds the first compile.
        let load = &code[code.find("def load_hearing(").unwrap()..code.find("def finite(").unwrap()];
        assert!(load.contains(".generate("), "load_hearing() must warm the model");
        // The model it loads for hearing is the one the runtime names, by path.
        assert!(code.contains("snapshot_download(state[\"stt_id\"], local_files_only=True)"));
        assert!(code.contains("load_stt(Path(path))"));
    }

    /// The script itself, run under the machine's `python3` with stand-ins for
    /// numpy and the model library (nothing real is loaded, nothing downloaded):
    /// `probe` is Python run after the script is imported as `m`, and prints
    /// one JSON line through `emit`'s own pipe. `None` where there is no `python3`.
    fn run_script(dir: &std::path::Path, probe: &str) -> Option<serde_json::Value> {
        let files: [(&str, &str); 6] = [
            ("numpy/__init__.py", "class _A:\n    def astype(self, t): return self\n    def __truediv__(self, o): return self\nfloat32 = 'float32'\ndef frombuffer(b, dtype=None): return _A()\ndef zeros(n, dtype=None): return _A()\n"),
            ("huggingface_hub/__init__.py", "def snapshot_download(name, local_files_only=False):\n    assert local_files_only\n    return '/nowhere/' + name\n"),
            ("mlx_audio/__init__.py", ""),
            ("mlx_audio/stt/__init__.py", ""),
            ("mlx_audio/stt/utils.py", "import types\nclass Model:\n    def __init__(self): self.calls = []\n    def generate(self, audio, **kw):\n        self.calls.append(kw)\n        return types.SimpleNamespace(text='hi', language='zh', segments=[{'text': 'hi', 'no_speech_prob': float('nan'), 'avg_logprob': -0.2, 'compression_ratio': 1.0}])\ndef load_model(path):\n    return Model()\n"),
            ("mlx_voice.py", include_str!("../../runtime_py/mlx_voice.py")),
        ];
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let code = format!("import json, sys\nimport mlx_voice as m\n{probe}\n");
        let out = std::process::Command::new("python3")
            .arg("-c")
            .arg(code)
            .env("PYTHONPATH", dir)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .ok()?;
        assert!(out.status.success(), "the probe failed: {}", String::from_utf8_lossy(&out.stderr));
        // The script keeps the real stdout for its protocol; the probe speaks on it.
        Some(serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout))))
    }

    /// Rule 2 and 3, on the script itself: a hear before the load is the
    /// `not_loaded` error, never a load; the load warms the model once; a hear
    /// passes the language and prompt through and asks the library to print
    /// nothing; and the numbers of a segment are always JSON — NaN (which the
    /// library's own no-speech figure is, without its token) goes as null, as
    /// JSON has no NaN and the engine kills a sidecar whose reply it cannot read.
    #[test]
    fn the_script_loads_apart_from_hearing_and_sends_numbers_json_can_carry() {
        let dir = tempfile::tempdir().unwrap();
        let probe = r#"
state = {"stt": None, "stt_id": "some/model"}
try:
    m.hear({"pcm_b64": "AAA="}, state)
    refused = False
except m.NotLoaded:
    refused = True
m.load_hearing(state)
warmed = len(state["stt"].calls)
reply = m.hear({"pcm_b64": "AAA=", "language": "zh", "prompt": "P"}, state)
bare = m.hear({"pcm_b64": "AAA="}, state)
m._proto.write(json.dumps({
    "refused": refused, "warmed": warmed,
    "kw": state["stt"].calls[-2], "bare_kw": state["stt"].calls[-1],
    "reply": json.loads(json.dumps(reply, allow_nan=False)),
    "finite": [m.finite(float("nan")), m.finite(float("inf")), m.finite("x"), m.finite(None), m.finite(-0.5), m.finite("2")],
}))
m._proto.flush()
"#;
        let Some(got) = run_script(dir.path(), probe) else {
            eprintln!("SKIPPED: no python3 on this machine for the script probe");
            return;
        };
        assert_eq!(got["refused"], true, "a hear never loads");
        assert_eq!(got["warmed"], 1, "the load warms the model with one clip");
        assert_eq!(got["kw"], serde_json::json!({ "verbose": null, "language": "zh", "initial_prompt": "P" }));
        assert_eq!(got["bare_kw"], serde_json::json!({ "verbose": null }), "no hint, nothing forced");
        assert_eq!(got["reply"]["segments"][0]["no_speech_prob"], serde_json::Value::Null);
        assert_eq!(got["reply"]["segments"][0]["avg_logprob"], -0.2);
        assert_eq!(got["finite"], serde_json::json!([null, null, null, null, -0.5, 2.0]));
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
