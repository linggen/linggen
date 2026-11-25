//! Apple's own speech recognition, on this Mac, as her hearing — a Mac has no
//! choice to make and no model of ours to fetch.
//!
//! The recognition is done by `ling-hear` (`apple_speech/`, Swift), a helper
//! `ling` carries and runs as its own process ([`super::apple_helper`]): a
//! clip goes in on its stdin as samples and the words come back as one JSON
//! line. A helper that cannot start, or that the system stops, ends one
//! hearing and never the engine. The recognition is Apple's on-device one
//! only (`speech.swift`: SpeechTranscriber from macOS 26, else
//! SFSpeechRecognizer with `requiresOnDeviceRecognition`): nothing is sent to
//! Apple's servers, and nothing here writes the sound or opens a connection
//! (`a_hearing_has_no_way_to_keep_or_send_the_sound`).
//!
//! Whether she can hear is per language, and a clip names its language only
//! when it is heard. So `can_hear` says only that Apple's recognition is
//! here (and that a language is being downloaded, if one is); the clip's
//! own answer says the rest ([`Verdict`]). A language that this Mac has not
//! got is put on it the first time a clip in it is heard, by the helper's
//! `install` — a hear never downloads anything — and that press answers
//! [`Next::Downloading`] with one line; the next press, once it is there,
//! hears.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::stt::{Heard, SttProvider, Unavailable, RATE};
use crate::runtime::Attempt;

/// The env var a test world sets to run its own stand-in for the helper
/// (`Helper::from_env`); a person's engine never has it.
const HELPER_ENV: &str = "LINGGEN_HEARING_HELPER";

/// The least a hear waits for the helper, and the most a language's install
/// does (a few hundred MB over the person's own line).
const HEAR_WAIT_SECS: u64 = 30;
const INSTALL_WAIT_SECS: u64 = 30 * 60;

/// Where the helper is: carried inside `ling`, or a program a test names.
#[derive(Clone)]
pub enum Helper {
    Embedded,
    At(PathBuf),
}

impl Helper {
    /// The helper this engine runs: its own, unless a test world names one.
    pub fn from_env() -> Self {
        std::env::var_os(HELPER_ENV).map_or(Self::Embedded, |path| Self::At(path.into()))
    }

    async fn program(&self) -> anyhow::Result<PathBuf> {
        match self {
            Self::At(path) => Ok(path.clone()),
            Self::Embedded => tokio::task::spawn_blocking(super::apple_helper::on_disk).await?,
        }
    }
}

/// What the helper says of a language: the words of `speech.swift`'s `Code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    /// Heard (or, for `state` and `install`, on this Mac).
    Ready,
    /// Apple has the language; this Mac has not yet.
    NeedsDownload,
    /// Its model is being put on this Mac now.
    Downloading,
    /// Apple's on-device recognition does not have the language.
    Unsupported,
    /// Speech recognition is not allowed for Linggen (macOS before 26).
    NotAllowed,
    /// Before macOS 26: the language's on-device model is not on this Mac; System Settings adds it.
    NeedsSettings,
    Failed,
}

#[derive(Debug, Deserialize)]
struct Reply {
    code: Verdict,
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: String,
}

pub struct AppleSttProvider {
    helper: Helper,
    /// The install of each language that has been asked for, by the tag the clip named.
    installs: Mutex<HashMap<String, Attempt>>,
}

impl AppleSttProvider {
    pub fn new(helper: Helper) -> Self {
        Self { helper, installs: Mutex::new(HashMap::new()) }
    }

    /// Run the helper once: `verb`, the language tag, and `sound` on its stdin.
    async fn ask(&self, verb: &str, language: &str, sound: Vec<u8>, wait: Duration) -> anyhow::Result<Reply> {
        let program = self.helper.program().await?;
        let mut child = Command::new(program)
            .args([verb, language])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?;
        // A helper that stops early closes the pipe: its own exit tells the rest.
        tokio::spawn(async move {
            let _ = stdin.write_all(&sound).await;
        });
        let output = tokio::time::timeout(wait, child.wait_with_output()).await??;
        let line = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(line.lines().next().unwrap_or("")).map_err(|e| anyhow::anyhow!("the helper's answer: {e}"))
    }

    /// Put the language on this Mac, unless that is already under way. The attempt is its own
    /// task, so a press that is dropped never abandons the download half way.
    /// Whether the last try at it failed, which the line says ([`lines::preparing_again`]).
    fn begin_install(&self, language: &str) -> bool {
        let attempt = {
            let mut installs = self.installs.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            installs.entry(language.to_ascii_lowercase()).or_default().clone()
        };
        let after_a_failure = attempt.state() == crate::runtime::AttemptState::Failed;
        let Some(run) = attempt.begin() else { return false };
        let (helper, language) = (self.helper.clone(), language.to_string());
        tokio::spawn(crate::runtime::run_attempt(
            run,
            async move {
                let provider = AppleSttProvider::new(helper);
                let reply = provider.ask("install", &language, Vec::new(), Duration::from_secs(INSTALL_WAIT_SECS)).await?;
                anyhow::ensure!(reply.code == Verdict::Ready, "installing {language}: {:?}", reply.code);
                Ok(())
            },
            || (),
        ));
        after_a_failure
    }

    /// The language whose install is running, if any.
    fn installing(&self) -> Option<String> {
        let installs = self.installs.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        installs
            .iter()
            .find(|(_, attempt)| matches!(attempt.state(), crate::runtime::AttemptState::Running { .. }))
            .map(|(language, _)| language.clone())
    }
}

#[async_trait]
impl SttProvider for AppleSttProvider {
    fn can_hear(&self) -> Result<(), Unavailable> {
        match self.installing() {
            Some(language) => Err(Unavailable::downloading(lines::preparing(&language))),
            None => Ok(()),
        }
    }

    /// Nothing to begin: what hearing waits on is a language's model, and which language is
    /// known only when a clip is heard (`hear` begins it).
    fn begin_fetch(&self) {}

    /// At a start: the helper on disk, so the first press does not wait for it.
    fn warm_if_fetched(&self) {
        if matches!(self.helper, Helper::Embedded) {
            tokio::task::spawn_blocking(|| {
                if let Err(e) = super::apple_helper::on_disk() {
                    tracing::warn!("[stt] hearing helper: {e:#}");
                }
            });
        }
    }

    async fn hear(&self, samples: &[i16], language: &str) -> Result<Heard, Unavailable> {
        let sound: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let wait = Duration::from_secs(HEAR_WAIT_SECS + samples.len() as u64 / RATE as u64 / 2);
        let reply = self.ask("hear", language, sound, wait).await.map_err(|e| {
            tracing::warn!("[stt] {e:#}");
            Unavailable::retry(lines::stopped())
        })?;
        match reply.code {
            Verdict::Ready => Ok(Heard { text: reply.text, language: reply.language }),
            Verdict::NeedsDownload | Verdict::Downloading => {
                match self.begin_install(language) {
                    true => Err(Unavailable::downloading(lines::preparing_again(language))),
                    false => Err(Unavailable::downloading(lines::preparing(language))),
                }
            }
            Verdict::Unsupported => Err(Unavailable::never(lines::unsupported(language))),
            Verdict::NotAllowed => Err(Unavailable::retry(lines::allow_speech())),
            Verdict::NeedsSettings => Err(Unavailable::retry(lines::add_language(language))),
            Verdict::Failed => Err(Unavailable::retry(lines::stopped())),
        }
    }
}

/// The lines a person reads, in the language Linggen is set to (English or Chinese, the two it
/// is written in). The one place they are worded.
mod lines {
    /// A language's name in English and in Chinese. The tag a clip names, or none.
    fn names(tag: &str) -> (String, String) {
        const ZH: &[(&str, &str)] = &[
            ("en", "英语"),
            ("zh-Hans", "中文"),
            ("zh-Hant", "繁体中文"),
            ("ja", "日语"),
            ("ko", "韩语"),
            ("fr", "法语"),
            ("de", "德语"),
            ("es", "西班牙语"),
            ("pt", "葡萄牙语"),
            ("it", "意大利语"),
            ("ru", "俄语"),
        ];
        if tag.is_empty() {
            return ("this language".into(), "这门语言".into());
        }
        let tag = crate::language::normalize(tag);
        let zh = ZH.iter().find(|(t, _)| t.eq_ignore_ascii_case(&tag)).map_or(tag.clone(), |(_, zh)| zh.to_string());
        (crate::language::display_name(&tag), zh)
    }

    fn zh_person() -> bool {
        crate::language::normalize(&crate::language::effective()).starts_with("zh")
    }

    fn pick(en: String, zh: String) -> String {
        if zh_person() { zh } else { en }
    }

    pub fn preparing(language: &str) -> String {
        let (en, zh) = names(language);
        pick(format!("Getting ready to hear {en}…"), format!("正在准备{zh}识别…"))
    }

    pub fn preparing_again(language: &str) -> String {
        let (en, zh) = names(language);
        pick(format!("The last try failed; getting ready to hear {en} again…"), format!("上次没成功，正在重新准备{zh}识别…"))
    }

    pub fn unsupported(language: &str) -> String {
        let (en, zh) = names(language);
        pick(format!("Apple's on-device recognition cannot hear {en}."), format!("苹果的本机识别听不了{zh}。"))
    }

    pub fn add_language(language: &str) -> String {
        let (en, zh) = names(language);
        pick(
            format!("Add {en} in System Settings → Keyboard → Dictation, then hold again."),
            format!("请在「系统设置 → 键盘 → 听写」里添加{zh}，然后再按住说话。"),
        )
    }

    pub fn allow_speech() -> String {
        pick(
            "Allow Linggen under System Settings → Privacy & Security → Speech Recognition, then hold again.".into(),
            "请在「系统设置 → 隐私与安全性 → 语音识别」里允许 Linggen，然后再按住说话。".into(),
        )
    }

    pub fn stopped() -> String {
        pick("Hearing stopped for a moment. Try again.".into(), "听写刚刚中断了，请再试一次。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::api::stt::Next;

    /// A stand-in helper: a shell script that logs each call's verb and language to `calls`, counts the
    /// bytes it was handed on stdin, and answers as `answers` says (one `case` arm per verb).
    fn helper(dir: &std::path::Path, answers: &str) -> Helper {
        let path = dir.join("ling-hear");
        let script = format!(
            "#!/bin/sh\nbytes=$(wc -c | tr -d ' ')\necho \"$1 $2 $bytes\" >> \"{}\"\ncase \"$1\" in\n{answers}\nesac\n",
            dir.join("calls").display()
        );
        std::fs::write(&path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Helper::At(path)
    }

    fn calls(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls")).unwrap_or_default().lines().map(String::from).collect()
    }

    async fn settled(provider: &AppleSttProvider) {
        for _ in 0..100 {
            if provider.installing().is_none() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the install did not end");
    }

    #[tokio::test]
    async fn a_clip_is_handed_over_as_samples_and_the_words_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let provider = AppleSttProvider::new(helper(dir.path(), r#"hear) echo '{"code":"ready","text":"I won'"'"'t speak more English.","language":"en"}';;"#));
        let heard = provider.hear(&[100; 8_000], "en").await.unwrap();
        assert_eq!((heard.text.as_str(), heard.language.as_str()), ("I won't speak more English.", "en"));
        assert_eq!(calls(dir.path()), ["hear en 16000"], "8000 samples of 2 bytes, on stdin, under the language the clip named");
        assert!(provider.can_hear().is_ok());
    }

    /// The constraint of the whole change: hearing a clip never downloads a model.
    #[tokio::test]
    async fn a_missing_language_is_downloaded_only_by_the_install_a_press_begins() {
        let dir = tempfile::tempdir().unwrap();
        let provider = AppleSttProvider::new(helper(
            dir.path(),
            r#"hear) echo '{"code":"needs_download","text":""}';;
install) sleep 0.3; echo '{"code":"ready"}';;"#,
        ));
        let no = provider.hear(&[100; 8_000], "zh-Hans").await.unwrap_err();
        assert_eq!(no.next, Next::Downloading);
        assert!(no.is_working(), "a page that does not know the word reads fetching: true");
        assert!(no.reason.contains("Chinese") || no.reason.contains("中文"), "{}", no.reason);
        // Under way: the question is answered, not a second download started.
        assert_eq!(provider.can_hear().unwrap_err().next, Next::Downloading);
        let again = provider.hear(&[100; 8_000], "zh-Hans").await.unwrap_err();
        assert_eq!(again.next, Next::Downloading);
        settled(&provider).await;
        assert!(provider.can_hear().is_ok());
        let verbs: Vec<String> = calls(dir.path()).iter().map(|c| c.split(' ').next().unwrap().to_string()).collect();
        assert_eq!(verbs.iter().filter(|v| *v == "install").count(), 1, "{verbs:?}");
        assert_eq!(verbs.iter().filter(|v| *v == "hear").count(), 2, "{verbs:?}");
    }

    /// An install that fails (no network) is said once the next press begins it again, not hidden behind the first line.
    #[tokio::test]
    async fn a_failed_install_is_said_to_have_failed_when_a_press_tries_again() {
        let dir = tempfile::tempdir().unwrap();
        let provider = AppleSttProvider::new(helper(
            dir.path(),
            r#"hear) echo '{"code":"needs_download"}';;
install) echo '{"code":"failed"}';;"#,
        ));
        let first = provider.hear(&[100; 8_000], "ja").await.unwrap_err();
        settled(&provider).await;
        assert!(provider.can_hear().is_ok(), "no one is told a download runs when it ended");
        let second = provider.hear(&[100; 8_000], "ja").await.unwrap_err();
        assert_eq!(second.next, Next::Downloading);
        assert_ne!(first.reason, second.reason, "{} / {}", first.reason, second.reason);
        assert!(second.reason.contains("failed") || second.reason.contains("没成功"), "{}", second.reason);
    }

    #[tokio::test]
    async fn what_the_system_says_is_one_line_and_a_word_for_what_would_help() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            ("unsupported", Next::Never),
            ("not_allowed", Next::Retry),
            ("needs_settings", Next::Retry),
            ("failed", Next::Retry),
        ];
        for (code, next) in cases {
            let provider = AppleSttProvider::new(helper(dir.path(), &format!(r#"hear) echo '{{"code":"{code}"}}';;"#)));
            let no = provider.hear(&[100; 8_000], "ja").await.unwrap_err();
            assert_eq!(no.next, next, "{code}");
            assert!(!no.reason.is_empty() && no.reason.chars().count() <= 100, "{code}: {}", no.reason);
        }
    }

    #[tokio::test]
    async fn a_helper_that_will_not_run_ends_one_hearing_and_nothing_more() {
        let provider = AppleSttProvider::new(Helper::At("/nonexistent/ling-hear".into()));
        let no = provider.hear(&[100; 8_000], "en").await.unwrap_err();
        assert_eq!(no.next, Next::Retry);
        assert!(provider.can_hear().is_ok());
        let dir = tempfile::tempdir().unwrap();
        let mute = AppleSttProvider::new(helper(dir.path(), "hear) echo 'not json';;"));
        assert_eq!(mute.hear(&[100; 8_000], "en").await.unwrap_err().next, Next::Retry);
    }

    #[test]
    fn a_language_is_named_for_the_person_in_their_own_words() {
        // No setting in a unit test: the Mac's language decides which of the two is said.
        let line = lines::preparing("zh-Hans");
        assert!(line == "Getting ready to hear Simplified Chinese…" || line == "正在准备中文识别…", "{line}");
        assert!(lines::preparing("").contains("this language") || lines::preparing("").contains("这门语言"));
    }

    /// The Swift side keeps to Apple's on-device recognition and keeps the sound in memory.
    #[test]
    fn the_helper_hears_on_the_device_and_keeps_nothing() {
        let swift = include_str!("apple_speech/speech.swift").to_string() + include_str!("apple_speech/main.swift");
        assert!(swift.contains("requiresOnDeviceRecognition = true"), "before macOS 26 the server must never be used");
        for banned in [
            "URLSession", "URL(string", "SFSpeechURLRecognitionRequest", "DictationTranscriber", "write(to", "createFile",
            "FileManager", "forWritingAtPath", "NWConnection", "CFStream",
        ] {
            assert!(!swift.contains(banned), "the helper names `{banned}`: what is heard is never kept or sent");
        }
    }

    #[test]
    fn the_verdicts_are_the_words_the_helper_says() {
        let swift = include_str!("apple_speech/speech.swift");
        for (word, verdict) in [
            ("ready", Verdict::Ready),
            ("needs_download", Verdict::NeedsDownload),
            ("downloading", Verdict::Downloading),
            ("unsupported", Verdict::Unsupported),
            ("not_allowed", Verdict::NotAllowed),
            ("needs_settings", Verdict::NeedsSettings),
            ("failed", Verdict::Failed),
        ] {
            assert!(swift.contains(&format!("= \"{word}\"")), "speech.swift has no `{word}`");
            assert_eq!(serde_json::from_str::<Verdict>(&format!("\"{word}\"")).unwrap(), verdict);
        }
    }
}
