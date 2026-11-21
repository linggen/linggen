//! The spoken-line contract: what of her reply reaches her voice.

/// The spoken-line contract every engine-authored kickoff carries. A model
/// that has to reason before it answers may do so above a blank line; what
/// reaches her voice is the final paragraph alone. Stated to her, not
/// inferred — so a model that thinks in its output (deepseek-v4-flash on the
/// fallback chain, 2026-09-08) is following the rules, not leaking.
pub(crate) const SPOKEN_CONTRACT: &str =
    "If you need to think first, do it above a blank line: only your \
    final paragraph is spoken aloud, the rest is discarded.";
/// What an engine-started kickoff carries: the one way to speak. Anything
/// else she writes in that turn is dropped (`finalize_event`).
pub(crate) const SAY_CONTRACT: &str =
    "To speak, put one line in <say>…</say>. Otherwise leave it out.";
/// The word a reply under the contract says nothing with.
macro_rules! silent_word {
    () => {
        "SILENT"
    };
}
/// The spoken contract's word for "say nothing": a reply whose final
/// paragraph is this word is silence — never spoken, never a chat bubble.
/// The web UI mirrors it (`ui/src/lib/hiddenRows.mts`, locked by a test).
pub(crate) const SILENT: &str = silent_word!();
/// What an unasked kickoff adds: silence is an answer.
pub(super) const SILENT_CLAUSE: &str = concat!(
    "If nothing is worth saying, make that final paragraph exactly ",
    silent_word!(),
    "."
);

/// The marker on a prompt the engine wrote: kept for the model, never shown
/// as anyone's words (`doc/chat-spec.md`, hidden rows).
const HIDDEN: &str = "[HIDDEN]";

/// Where a turn of hers comes from, carried from where the turn starts. The
/// one fact that decides whether her reply is held for the finalizer, which
/// speaking contract the model reads, and which finalizer reads the reply —
/// nothing else decides any of the three.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Origin {
    /// A person wrote to her — or the turn is not hers at all (another
    /// agent's): the answer path, shown as it always was.
    Person,
    /// An answer a person asked for (an app's asked moment, a tapped offer's
    /// result): the answer path, with the spoken-line contract.
    Asked,
    /// Another agent finished a reply, or a question waits (a herald).
    Herald,
    /// A peer agent's message addressed to her.
    AgentChat,
    /// A mission run's or another agent's failure.
    Fault,
    /// An app moment, from an app the person is not using.
    Moment,
    /// An app moment from the app the person is using.
    InUse,
    /// An ambient glance, or a condition crossing.
    Glance,
    /// A look-again note that came due.
    Note,
    /// A background task's end told to her, or a restart's notice.
    TaskEnd,
}

impl Origin {
    /// Whether her reply is the finalizer's to decide: nothing of it is
    /// streamed, shown or kept by the turn itself, and nothing joins the turn.
    pub(crate) fn held(self) -> bool {
        !matches!(self, Self::Person | Self::Asked)
    }

    /// Whether this is a kind of turn the engine started for her; false for
    /// `Person`, the answer path (a person's words, or any other agent's
    /// turn). Exhaustive: a new origin is decided here.
    pub(crate) fn is_hers(self) -> bool {
        match self {
            Self::Person => false,
            Self::Asked
            | Self::Herald
            | Self::AgentChat
            | Self::Fault
            | Self::Moment
            | Self::InUse
            | Self::Glance
            | Self::Note
            | Self::TaskEnd => true,
        }
    }

    /// The origin's name, for the log and the activity log's `hold` record
    /// (`guard::Source::name` is this): one name per kind of turn.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Person => "user",
            Self::Asked => "asked",
            Self::Herald => "event",
            Self::AgentChat => "agent_chat",
            Self::Fault => "fault",
            Self::Moment => "moment",
            Self::InUse => "moment-in-use",
            Self::Glance => "glance",
            Self::Note => "note",
            Self::TaskEnd => "task end",
        }
    }

    /// The speaking contract a turn of this origin reads after its kickoff;
    /// a person's own words carry none.
    pub(crate) fn contract(self) -> Option<&'static str> {
        match self {
            Self::Person => None,
            Self::Asked => Some(SPOKEN_CONTRACT),
            _ => Some(SAY_CONTRACT),
        }
    }

    /// The kickoff as the thread keeps it: an engine-authored one is a hidden
    /// row (the engine wrote it, not the user); a person's words stay theirs.
    pub(crate) fn kickoff(self, task: String) -> String {
        match self {
            Self::Person => task,
            _ => {
                let task = task.trim_start_matches(HIDDEN).trim_start();
                format!("{HIDDEN} {task}")
            }
        }
    }

    /// The finalizer for a reply of this origin: an answer for a person, the
    /// envelope for everything the engine started. The one place that picks.
    fn finalize(self, reply: Option<&str>) -> Option<Event> {
        match self.held() {
            true => finalize_event(reply, self.name()),
            false => finalize_answer(reply),
        }
    }
}

impl Origin {
    /// The origin of a turn told to `agent`: only her turns have one but
    /// `Person` — any other agent's turn is answered as ever. `told`: what
    /// the telling is (a task's end, or the turn a restart continues).
    pub(crate) fn for_agent(agent: &str, told: Origin) -> Origin {
        match agent == crate::engine::agent::COMPANION_AGENT_ID {
            true => told,
            false => Self::Person,
        }
    }
}

/// The turns of hers running now, one entry each, keyed by the turn — two
/// overlapping turns in a session never clear each other's. Recorded when the
/// turn takes its place in the session's line, so there is no moment a held
/// turn runs unrecorded. A restart's plan reads the origin from here.
static RUNNING: std::sync::Mutex<Vec<Running>> = std::sync::Mutex::new(Vec::new());
static NEXT_TURN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

struct Running {
    turn: u64,
    session: String,
    origin: Origin,
    /// A person's message ended this turn: what it says is dropped.
    ended: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Held while a turn of `origin` runs in `session`; removed by its own turn.
pub(crate) struct RunningOrigin {
    turn: u64,
    ended: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl RunningOrigin {
    /// Recorded the moment a turn of hers the engine started takes the
    /// session's turn lock — before anything of it runs. A person's turn is
    /// recorded by [`Self::in_turn`] instead, so none is recorded twice.
    ///
    /// A person's message waiting for the session ([`PersonWaiting`]) outranks
    /// it: a turn that takes the lock ahead of that message is ended on
    /// arrival — it does not run, and nothing of it is published.
    pub(crate) fn at_lock(session: &str, origin: Origin) -> Option<Self> {
        let held = origin.held().then(|| Self::keep(session, origin))?;
        if person_is_waiting(session) {
            held.ended.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Some(held)
    }

    /// A person's turn, recorded as it starts running.
    pub(crate) fn in_turn(session: &str, origin: Origin) -> Option<Self> {
        (!origin.held()).then(|| Self::keep(session, origin))
    }

    fn keep(session: &str, origin: Origin) -> Self {
        use std::sync::atomic::Ordering;
        let turn = NEXT_TURN.fetch_add(1, Ordering::Relaxed);
        let ended = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut all = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
        all.push(Running { turn, session: session.to_string(), origin, ended: ended.clone() });
        Self { turn, ended }
    }

    /// Whether a person's message ended this turn.
    pub(crate) fn was_ended(&self) -> bool {
        self.ended.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Drop for RunningOrigin {
    fn drop(&mut self) {
        let mut all = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
        all.retain(|r| r.turn != self.turn);
    }
}

/// Sessions where a person's message waits for the turn lock behind a held
/// turn, one entry per waiting message.
static WAITING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Held while a person's message waits for `session`; dropped when it has
/// the lock (or gives up).
pub(crate) struct PersonWaiting(String);

impl PersonWaiting {
    pub(crate) fn new(session: &str) -> Self {
        WAITING.lock().unwrap_or_else(|p| p.into_inner()).push(session.to_string());
        Self(session.to_string())
    }
}

impl Drop for PersonWaiting {
    fn drop(&mut self) {
        let mut all = WAITING.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(at) = all.iter().position(|s| *s == self.0) {
            all.remove(at);
        }
    }
}

fn person_is_waiting(session: &str) -> bool {
    WAITING.lock().unwrap_or_else(|p| p.into_inner()).iter().any(|s| s == session)
}

/// The origin of the newest turn running in `session`, when one is.
pub(crate) fn running_origin(session: &str) -> Option<Origin> {
    let all = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
    all.iter().rev().find(|r| r.session == session).map(|r| r.origin)
}

/// A person's message ends every held turn running in `session`: each is
/// marked, and what it says is dropped. True when there was one.
pub(crate) fn end_held_turns(session: &str) -> bool {
    let all = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
    let mut any = false;
    for r in all.iter().filter(|r| r.session == session && r.origin.held()) {
        r.ended.store(true, std::sync::atomic::Ordering::Relaxed);
        any = true;
    }
    any
}

/// The runs of held turns, newest last — kept past their turn, so the run's
/// last events (its outcome, its turn end) are still known as held.
static HELD_RUNS: std::sync::Mutex<std::collections::VecDeque<String>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());
const HELD_RUNS_KEPT: usize = 256;

/// Run `run_id` is a held turn's (`chat::runtime`, as the run begins).
pub(crate) fn hold_run(run_id: &str) {
    let mut runs = HELD_RUNS.lock().unwrap_or_else(|p| p.into_inner());
    runs.push_back(run_id.to_string());
    while runs.len() > HELD_RUNS_KEPT {
        runs.pop_front();
    }
}

/// Whether a page draws an event of `kind` that `agent` sent in `session`
/// for `run`: the one decision that a turn the engine started for her shows
/// nothing on any page — no status, no run, no token count, live or after.
/// What she says reaches the page as her stage's cue (`pet_*`) or her own
/// row; everything her held run sends is dropped. A status sent before the
/// run has an id is known by the held turn running in its session.
pub(crate) fn shows_on_page(kind: &str, agent: Option<&str>, session: Option<&str>, run: Option<&str>) -> bool {
    if agent != Some(crate::engine::agent::COMPANION_AGENT_ID) || kind.starts_with("pet_") {
        return true;
    }
    if let Some(run) = run {
        let runs = HELD_RUNS.lock().unwrap_or_else(|p| p.into_inner());
        return !runs.iter().any(|r| r == run);
    }
    let before_run = kind == crate::server::ui_events::UI_KIND_ACTIVITY;
    !(before_run && session.and_then(running_origin).is_some_and(Origin::held))
}

/// The kickoff marked hidden and with the speaking contract its origin names,
/// in one text — for the light turn, which has no thread message to carry the
/// contract. A full turn has it attached by `ChatRunCtx::labeled_msg`.
pub(crate) fn with_contract(task: String, origin: Origin) -> String {
    let task = origin.kickoff(task);
    match origin.contract() {
        Some(contract) => format!("{task} {contract}"),
        None => task,
    }
}

/// A row that is a prompt the engine wrote, not anyone's words: it carries
/// the hidden marker — or, on a row saved before her kickoffs were marked
/// (2026-10-14), the spoken contract every engine kickoff has carried since
/// 2026-09-08 and no person's message does. Kept for the model; the chat
/// never shows it.
pub(crate) fn is_engine_prompt(from_id: &str, content: &str) -> bool {
    content.contains(HIDDEN)
        || (crate::state_fs::RowAuthor::of(from_id).is_person() && (content.contains(SPOKEN_CONTRACT) || content.contains(SAY_CONTRACT)))
}

/// What a reply to a PERSON (or to an answer they asked for) says: the final
/// paragraph, unwrapped from quotes. `None` when it is — or ends in — SILENT.
/// The answer path only: a turn the engine started is read by [`spoken_line`].
pub(crate) fn answer_line(reply: &str) -> Option<String> {
    // Her labels on a problem's offers are buttons, never words to say.
    let reply = super::offer_labels::strip(reply);
    let paragraphs: Vec<&str> = reply
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let last = paragraphs.last()?;
    let notes: usize = paragraphs[..paragraphs.len() - 1]
        .iter()
        .map(|p| p.len())
        .sum();

    // Her voice says the words, never a picture's or a source's number.
    let last = crate::engine::answer_refs::strip_ids(last);
    let mut line = last
        .split('\n')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    for (open, close) in [
        ('"', '"'),
        ('\u{201c}', '\u{201d}'),
        ('\u{2018}', '\u{2019}'),
        ('\u{300c}', '\u{300d}'),
    ] {
        if line.starts_with(open) && line.ends_with(close) && line.chars().count() > 1 {
            line = line[open.len_utf8()..line.len() - close.len_utf8()]
                .trim()
                .to_string();
        }
    }

    let core = line.trim_end_matches(is_punct);
    let last_word = core.split_whitespace().last().unwrap_or("");
    if is_silence(core) || last_word.trim_end_matches(is_punct) == SILENT {
        return None;
    }
    if notes > 0 {
        tracing::info!(
            "[yinyue] spoke the final paragraph; {notes} chars of notes above it discarded"
        );
    }
    Some(line)
}

fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || c == '\u{2026}'
}

/// One paragraph that is the contract's silence: the word alone, any case,
/// trailing punctuation aside.
fn is_silence(paragraph: &str) -> bool {
    paragraph
        .trim()
        .trim_end_matches(is_punct)
        .eq_ignore_ascii_case(SILENT)
}

/// An agent's reply that is silence: its whole final paragraph is the
/// contract's [`SILENT`]. Nothing was said — the chat shows no words for it
/// (its tool calls may stay). A person's message is never one.
pub(crate) fn is_silent_reply(from_id: &str, content: &str) -> bool {
    use crate::state_fs::RowAuthor;
    if matches!(RowAuthor::of(from_id), RowAuthor::Person | RowAuthor::System) {
        return false;
    }
    content
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .last()
        .is_some_and(is_silence)
}

/// What a turn of hers said, with the origin the turn STARTED as — the one
/// value every reader of a reply takes. Its finalizer is picked by that
/// origin alone: a reply cannot be read as another kind of turn's.
#[derive(Debug, Clone)]
pub(crate) struct Turned {
    text: String,
    origin: Origin,
}

/// A reply to a person (or to an answer they asked for) that may be voiced:
/// only a person-origin [`Turned`] makes one.
pub(crate) struct Answer(Line);

impl Turned {
    /// `text` as the turn of `origin` ended — `origin` the turn's own value,
    /// never a variant named where the reply is read (the lock test says so).
    pub(crate) fn of(text: String, origin: Origin) -> Self {
        Self { text, origin }
    }

    pub(crate) fn chars(&self) -> usize {
        self.text.chars().count()
    }

    /// The decision of a turn the engine started — or, for a person's, the
    /// answer path's; picked by the origin.
    pub(super) fn finalize(&self) -> Option<Event> {
        self.origin.finalize(Some(&self.text))
    }

    /// Her reply to a person, read as ever: its final paragraph. `None` for
    /// a turn the engine started, and for SILENT.
    pub(crate) fn answer(&self) -> Option<Answer> {
        if self.origin.held() {
            return None;
        }
        answer_line(&self.text).map(|line| Answer(Line::new(line)))
    }

    /// Her reply to a person who talked to her on her own stage, as it
    /// always was: the whole reply, unless it is the bare word SILENT.
    pub(crate) fn whole_answer(&self) -> Option<Answer> {
        if self.origin.held() || self.text.eq_ignore_ascii_case(SILENT) {
            return None;
        }
        Some(Answer(Line::new(self.text.clone())))
    }
}

/// Say an answer to a person. `at`: where she said it — whether it is heard
/// or only read is the voice's own rule (`emit_speak`).
pub(crate) fn speak_reply(
    state: &std::sync::Arc<crate::server::ServerState>,
    answer: Answer,
    at: crate::server::api::yinyue::SaidAt<'_>,
) {
    crate::server::api::yinyue::emit_speak(state, answer.0, None, at);
}

/// The most a spoken line may be, in characters: an app moment's text is cut
/// at the same 300 (`api::yinyue::EVENT_TEXT_MAX`), and a phone notice's body
/// at 320 (`push::compose`) — a line the voice is to say is shorter than both.
pub(crate) const LINE_MAX: usize = 300;

const SAY_OPEN: &str = "<say>";
const SAY_CLOSE: &str = "</say>";

/// Why a reply of an engine-started turn said nothing. Logged by name, never
/// with the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejected {
    NoEnvelope,
    Several,
    Unclosed,
    TextAfter,
    Empty,
    LineBreak,
    TooLong,
}

impl Rejected {
    fn name(self) -> &'static str {
        match self {
            Self::NoEnvelope => "no envelope",
            Self::Several => "more than one envelope",
            Self::Unclosed => "unclosed envelope",
            Self::TextAfter => "text after the envelope",
            Self::Empty => "empty envelope",
            Self::LineBreak => "line break inside the envelope",
            Self::TooLong => "longer than a spoken line",
        }
    }
}

/// The one `<say>…</say>` of an engine-started turn's reply: what is inside
/// it and what follows it. Text before it (a model thinking first) is not
/// returned — nothing there is read for anything. Two envelopes, an unclosed
/// or a stray one, or none, is nothing.
fn envelope(reply: &str) -> Result<(&str, &str), Rejected> {
    let opens = reply.matches(SAY_OPEN).count();
    let closes = reply.matches(SAY_CLOSE).count();
    if opens == 0 && closes == 0 {
        return Err(Rejected::NoEnvelope);
    }
    if opens > 1 || closes > 1 {
        return Err(Rejected::Several);
    }
    let (Some(open), Some(close)) = (reply.find(SAY_OPEN), reply.find(SAY_CLOSE)) else {
        return Err(Rejected::Unclosed);
    };
    if close < open {
        return Err(Rejected::Unclosed);
    }
    Ok((&reply[open + SAY_OPEN.len()..close], &reply[close + SAY_CLOSE.len()..]))
}

/// The line an engine-started turn's reply speaks, with her labels on the
/// offers it raised; `Err` says why it is nothing. The envelope must be the
/// last thing in the reply: only her offer labels (metadata) may follow it.
/// Labels count inside the envelope or directly after it — nothing before
/// the envelope is read.
fn read_envelope(reply: &str) -> Result<(String, Vec<(usize, String)>), Rejected> {
    let (inside, after) = envelope(reply)?;
    let (after, mut labels) = super::offer_labels::take(after);
    if !after.is_empty() {
        return Err(Rejected::TextAfter);
    }
    let (inside, within) = super::offer_labels::take(inside);
    labels.splice(0..0, within);
    let line = crate::engine::answer_refs::strip_ids(&inside);
    let line = line.trim();
    if line.is_empty() {
        return Err(Rejected::Empty);
    }
    if line.contains('\n') {
        return Err(Rejected::LineBreak);
    }
    if line.chars().count() > LINE_MAX {
        return Err(Rejected::TooLong);
    }
    Ok((line.to_string(), labels))
}

/// A line of hers that may be published: the one type the row, the live
/// event and the voice take (`api::yinyue::emit_speak_offering`,
/// [`publish`]). Its field and its constructor are private to this file, so
/// outside it a `Line` can only come from a finalizer ([`finalize_event`],
/// [`finalize_answer`], [`speak_reply`], [`speak_person_reply`]) — no other
/// path can put words in her mouth.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Line {
    text: String,
}

impl Line {
    fn new(text: String) -> Self {
        Self { text }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

/// What a finalizer hands back: the line she chose to say, and the labels
/// she gave the offers it raised. `kept`: the turn itself already kept the
/// line as her row (an answer to a person) — publication adds only the cue.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Event {
    pub line: Line,
    pub labels: Vec<(usize, String)>,
    pub kept: bool,
}

/// An answer a person asked for, read as it always was (final paragraph, or
/// SILENT): the answer path, never an event's.
pub(super) fn finalize_answer(reply: Option<&str>) -> Option<Event> {
    let reply = reply?;
    let line = answer_line(reply)?;
    Some(Event {
        line: Line::new(line),
        labels: super::offer_labels::take(reply).1,
        kept: true,
    })
}

/// The one decision for every turn the engine started: did she choose to
/// speak? `source` names the wake, for the log. Nothing else she wrote in the
/// turn is shown, kept or spoken.
pub(super) fn finalize_event(reply: Option<&str>, source: &str) -> Option<Event> {
    let reply = reply?;
    match read_envelope(reply) {
        Ok((text, labels)) => Some(Event {
            line: Line::new(text),
            labels,
            kept: false,
        }),
        Err(why) => {
            tracing::info!(
                "[yinyue] {source}: nothing said ({}; {} chars dropped)",
                why.name(),
                reply.chars().count()
            );
            None
        }
    }
}

/// The language of a line a person wrote, when it can be told from the line:
/// Chinese when Han characters outnumber its Latin words. Anything else says
/// nothing — the turn then keeps the UI / selected app language the engine
/// already knows. The sample is the person's, never the English kickoff.
pub(crate) fn language_of_sample(sample: &str) -> Option<String> {
    let han = crate::server::api::tts_text::han_count(sample);
    let latin = sample
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| w.len() >= 2)
        .count();
    (han > 0 && han >= latin).then(|| "zh-Hans".to_string())
}

/// Where a published line stands: the session whose thread keeps it as her
/// row, the place the row names (`None`: the store stamps the running
/// turn's, as for any row), and what her stage is told.
pub(super) struct Stage {
    row_session: String,
    place: Option<String>,
    said_session: Option<String>,
    app: Option<String>,
    /// The caller holds the row session's turn lock: the row is written
    /// under it, never waited for.
    holds_lock: bool,
}

impl Stage {
    /// Her own thread (`table` none) or the app chat a moment names.
    pub(super) async fn of(state: &std::sync::Arc<crate::server::ServerState>, table: Option<&super::Table>) -> Self {
        let Some(table) = table else {
            let row_session = super::session_roll::resolve_current_session(state).await;
            return Self { row_session, place: None, said_session: None, app: None, holds_lock: false };
        };
        let app = super::triggers::table_page(state, table);
        // At an app's table the row names the place the turn stood in, as the
        // turn core wrote it before; her own thread's row names none.
        let place = Some(super::turn::table_place(state, table).key().to_string());
        Self { row_session: table.session.clone(), place, said_session: Some(table.session.clone()), app, holds_lock: false }
    }

    /// Whether her row could land this moment: no turn runs in the row's
    /// session. A condition is published only then (it is told only with its
    /// row kept), so it is not even attempted while the session is busy.
    pub(super) fn row_is_free(&self, state: &crate::server::ServerState) -> bool {
        self.holds_lock || state.manager.session_turn(&self.row_session).try_lock_owned().is_ok()
    }

    /// A turn the caller ran in `session` and still holds the turn lock of:
    /// its place is the row's.
    pub(super) fn of_turn(session: &str, place: &crate::engine::turn_place::TurnPlace) -> Self {
        Self {
            row_session: session.to_string(),
            place: Some(place.key().to_string()),
            said_session: Some(session.to_string()),
            app: place.skill.clone(),
            holds_lock: true,
        }
    }

    pub(super) fn at(&self) -> crate::server::api::yinyue::SaidAt<'_> {
        crate::server::api::yinyue::SaidAt { session: self.said_session.as_deref(), app: self.app.as_deref() }
    }
}

/// The one publication of an engine-started turn's line: her row in the
/// thread, and her stage (the live cue, the bubble, the voice). Every output
/// takes the finalizer's typed [`Line`]; staleness is the caller's check,
/// made before this is called.
///
/// Her row lands when the session is free: while a turn runs there (a
/// question waits inside it) the cue goes now and the row follows when the
/// turn is over — except a condition (`problem`), which is published only
/// with its row kept, so it can be told. `None`: nothing was published.
/// The window between the caller's last check and the row is accepted: two
/// awaits at most, and the worst is a line a moment late. A routine herald's
/// row that waits for a turn may land after the person's next message — as on
/// main, accepted: the cue is time-critical, the row is the record.
pub(super) async fn publish(
    state: &std::sync::Arc<crate::server::ServerState>,
    said: Event,
    stage: &Stage,
    emotion: Option<String>,
    problem: Option<(String, Vec<crate::perception::offer::Shown>)>,
) -> Option<Line> {
    let Event { line, labels, kept } = said;
    let row = (stage.row_session.as_str(), stage.place.as_deref());
    let mut later = false;
    if !kept && !stage.holds_lock {
        match state.manager.session_turn(&stage.row_session).try_lock_owned() {
            Ok(_turn) => {
                if !keep_row(state, &line, &labels, row).await {
                    return None;
                }
            }
            Err(_) if problem.is_some() => {
                tracing::info!("[yinyue] a condition's row cannot land while a turn runs — held, not told");
                return None;
            }
            Err(_) => later = true,
        }
    } else if !kept && !keep_row(state, &line, &labels, row).await {
        return None;
    }
    crate::server::api::yinyue::emit_speak_offering(state, line.clone(), emotion, stage.at(), problem);
    if later {
        keep_row_when_free(state.clone(), line.clone(), labels, (stage.row_session.clone(), stage.place.clone()));
    }
    Some(line)
}

/// Her line copied into another chat an app moment names: a row there, no cue.
pub(crate) async fn land_row(state: &std::sync::Arc<crate::server::ServerState>, line: &Line, session: &str) {
    keep_row(state, line, &[], (session, None)).await;
}

/// A turn of hers the engine started in `session` (a told task end), finalized
/// and published where it stood; the caller holds the session's turn lock.
pub(crate) async fn say_event(
    state: &std::sync::Arc<crate::server::ServerState>,
    reply: Option<&Turned>,
    (session, place): (&str, &crate::engine::turn_place::TurnPlace),
) {
    let Some(said) = reply.and_then(Turned::finalize) else {
        return;
    };
    let stage = Stage::of_turn(session, place);
    publish(state, said, &stage, Some("neutral".to_string()), None).await;
}

/// The row of a cue that went out while a turn ran: kept once it is over.
fn keep_row_when_free(
    state: std::sync::Arc<crate::server::ServerState>,
    line: Line,
    labels: Vec<(usize, String)>,
    (session, place): (String, Option<String>),
) {
    tokio::spawn(async move {
        let _turn = state.manager.session_turn(&session).lock_owned().await;
        keep_row(&state, &line, &labels, (&session, place.as_deref())).await;
    });
}

/// Her line as a row of the thread, her labels kept on it as the record of
/// what she offered (the UI strips them out of the words) — and the live
/// message of it. The one writer of her rows and messages: private to this
/// file. True when the row was kept.
async fn keep_row(
    state: &std::sync::Arc<crate::server::ServerState>,
    line: &Line,
    labels: &[(usize, String)],
    (session, place): (&str, Option<&str>),
) -> bool {
    let mut content = line.text().to_string();
    for (n, label) in labels {
        content.push_str(&format!(" [offer {n}: {label}]"));
    }
    let her = crate::engine::agent::COMPANION_AGENT_ID;
    let msg = crate::state_fs::sessions::ChatMsg {
        run_id: None,
        via: None,
        via_note: None,
        agent_id: her.to_string(),
        from_id: her.to_string(),
        to_id: "user".to_string(),
        content: content.clone(),
        timestamp: crate::util::now_ts_secs(),
        is_observation: false,
        client_id: None,
        failed: false,
        images: Vec::new(),
        place: place.map(str::to_string),
    };
    if let Err(e) = state.manager.global_sessions.add_chat_message(session, &msg) {
        tracing::warn!("[yinyue] her line was not kept as a row: {e}");
        return false;
    }
    let _ = state.events_tx.send(crate::engine::events::ServerEvent::Message {
        from: her.to_string(),
        to: "user".to_string(),
        content,
        session_id: Some(session.to_string()),
        run_id: None,
        parent_agent_id: None,
        client_id: None,
        via: None,
        via_note: None,
        images: Vec::new(),
    });
    true
}

/// [`finalize_event`]'s line, bare — for the tests that only ask what is said.
#[cfg(test)]
pub(super) fn spoken_line(reply: &str) -> Option<String> {
    finalize_event(Some(reply), "test").map(|e| e.line.text)
}

#[cfg(test)]
mod tests {
    use super::{answer_line, is_engine_prompt, spoken_line, with_contract, Origin};

    use super::{end_held_turns, PersonWaiting, RunningOrigin};

    /// A held turn that takes the lock while a person's message waits behind
    /// the running one is ended on arrival; one that takes it otherwise is not.
    #[test]
    fn a_held_turn_taking_the_lock_ahead_of_a_waiting_person_is_ended() {
        let session = "waiting-test-session";
        let first = RunningOrigin::at_lock(session, Origin::Note).unwrap();
        assert!(!first.was_ended());
        let waiting = PersonWaiting::new(session);
        assert!(end_held_turns(session));
        assert!(first.was_ended());
        let queued = RunningOrigin::at_lock(session, Origin::Herald).unwrap();
        assert!(queued.was_ended(), "a second held turn ran ahead of the person");
        let elsewhere = RunningOrigin::at_lock("another-session", Origin::Herald).unwrap();
        assert!(!elsewhere.was_ended());
        drop(waiting);
        let later = RunningOrigin::at_lock(session, Origin::Glance).unwrap();
        assert!(!later.was_ended(), "the person already has the session");
        assert!(RunningOrigin::at_lock(session, Origin::Person).is_none());
    }

    /// A held run of hers reaches no page: none of its events, whatever the
    /// kind; her stage's cues, her other runs and anyone else's pass.
    #[test]
    fn a_held_run_of_hers_shows_on_no_page() {
        use super::{hold_run, shows_on_page};
        hold_run("held-run-1");
        for kind in ["activity", "run", "turn_complete", "token", "message", "content_block"] {
            assert!(!shows_on_page(kind, Some("yinyue"), Some("s"), Some("held-run-1")), "{kind}");
        }
        assert!(shows_on_page("pet_speak", Some("yinyue"), Some("s"), Some("held-run-1")));
        assert!(shows_on_page("activity", Some("yinyue"), Some("s"), Some("her-asked-run")));
        assert!(shows_on_page("activity", Some("ling"), Some("s"), Some("held-run-1")));
        // Before the run has an id: known by the held turn in the session.
        let turn = RunningOrigin::at_lock("held-turn-session", Origin::Glance).unwrap();
        assert!(!shows_on_page("activity", Some("yinyue"), Some("held-turn-session"), None));
        assert!(shows_on_page("message", Some("yinyue"), Some("held-turn-session"), None), "her own row");
        drop(turn);
        assert!(shows_on_page("activity", Some("yinyue"), Some("held-turn-session"), None));
    }

    #[test]
    fn an_asked_turn_is_not_offered_silence() {
        let event = with_contract("k".into(), Origin::Herald);
        assert!(event.contains("<say>") && !event.contains("SILENT"));
        let asked = with_contract("k".into(), Origin::Asked);
        assert!(asked.contains("final paragraph is spoken"));
        assert!(!asked.contains("SILENT"));
        assert_eq!(with_contract("hi".into(), Origin::Person), "hi");
    }

    #[test]
    fn an_engine_kickoff_is_hidden_a_persons_words_are_not() {
        assert!(with_contract("k".into(), Origin::Herald).starts_with("[HIDDEN] k "));
        assert!(with_contract("k".into(), Origin::Asked).starts_with("[HIDDEN] k "));
        // Already marked: one marker, not two.
        let once = with_contract("[HIDDEN] k".into(), Origin::Herald);
        assert_eq!(once.matches("[HIDDEN]").count(), 1);
        assert!(!with_contract("hi".into(), Origin::Person).contains("[HIDDEN]"));
    }

    #[test]
    fn an_old_kickoff_row_is_known_by_its_contract() {
        let kickoff = with_contract("A quiet moment".into(), Origin::Moment);
        assert!(is_engine_prompt("user", &kickoff));
        // A row saved before the marker: the contract alone names it.
        let old = kickoff.trim_start_matches("[HIDDEN] ");
        assert!(is_engine_prompt("user", old));
        assert!(!is_engine_prompt("user", "hello"));
        assert!(!is_engine_prompt(
            "user",
            &with_contract("hello".into(), Origin::Person)
        ));
    }

    #[test]
    fn a_silent_reply_is_its_whole_final_paragraph() {
        use super::is_silent_reply;
        assert!(is_silent_reply("yinyue", "SILENT"));
        assert!(is_silent_reply("ling", "  SILENT.\n"));
        assert!(is_silent_reply("yinyue", "Let me think.\n\nSILENT"));
        assert!(!is_silent_reply(
            "yinyue",
            "It is quiet. SILENT is my answer?"
        ));
        assert!(!is_silent_reply("yinyue", "Silent night tonight."));
        assert!(!is_silent_reply("user", "SILENT"));
        assert!(super::SILENT_CLAUSE.ends_with("exactly SILENT."));
    }

    #[test]
    fn the_web_ui_mirrors_the_silent_word() {
        let ui = include_str!("../../../ui/src/lib/hiddenRows.mts");
        let mirror = format!("export const SILENT = '{}';", super::SILENT);
        assert!(ui.contains(&mirror), "hiddenRows.mts must declare {mirror}");
    }

    #[test]
    fn event_speech_requires_one_final_envelope() {
        for reply in [
            "A reply is ready.", "他不在，先不说。", "<say>Unclosed",
            "<say></say>", "<say>Hi</say> more", "<say>Hi\nthere</say>",
            "<say>First</say><say>Second</say>",
        ] {
            assert_eq!(spoken_line(reply), None, "{reply:?}");
        }
        assert_eq!(spoken_line("Thinking. <say>好了。</say> \n").as_deref(), Some("好了。"));
        assert_eq!(spoken_line(&format!("<say>{}</say>", "字".repeat(301))), None);
    }

    #[test]
    fn the_envelope_limit_and_offer_labels_are_metadata() {
        use super::{finalize_event, LINE_MAX};
        let at_limit = format!("<say>{}</say>", "字".repeat(LINE_MAX));
        assert!(spoken_line(&at_limit).is_some());
        assert_eq!(spoken_line("</say>x<say>"), None);
        assert_eq!(spoken_line("<say>Hi</say></say>"), None);
        // Labels ride beside the line and are never part of it.
        let e = finalize_event(Some("<say>Full. [offer 1: Open Files]</say>"), "t").unwrap();
        assert_eq!(e.line.text(), "Full.");
        assert_eq!(e.labels, vec![(1, "Open Files".to_string())]);
        // A label outside the envelope, after it, is still metadata.
        let e = finalize_event(Some("<say>Full.</say> [offer 1: Open]"), "t").unwrap();
        assert_eq!((e.line.text(), e.labels.len()), ("Full.", 1));
        // A bare reply, or SILENT, says nothing.
        assert!(finalize_event(Some("SILENT"), "t").is_none());
        assert!(finalize_event(None, "t").is_none());
    }

    /// Nothing before the envelope is read for anything: a label there was
    /// dropped with the rest of the thinking. Labels count inside the envelope
    /// or directly after it.
    #[test]
    fn a_label_before_the_envelope_is_not_read() {
        use super::finalize_event;
        let e = finalize_event(Some("Thinking [offer 1: Sneaky] now. <say>Full.</say>"), "t").unwrap();
        assert_eq!(e.line.text(), "Full.");
        assert!(e.labels.is_empty(), "{:?}", e.labels);
        // Only labels may follow the envelope; any other text rejects it.
        assert!(finalize_event(Some("<say>Full.</say> [offer 1: Open] and more"), "t").is_none());
    }

    /// Where a turn comes from is one typed fact: what is held, which contract
    /// it reads and which finalizer reads its reply all follow from it.
    #[test]
    fn the_origin_alone_decides_held_contract_and_finalizer() {
        use super::Origin::*;
        for held in [Herald, AgentChat, Fault, Moment, InUse, Glance, Note, TaskEnd] {
            assert!(held.held(), "{held:?}");
            assert_eq!(held.contract(), Some(super::SAY_CONTRACT), "{held:?}");
            assert!(held.finalize(Some("plain prose")).is_none(), "{held:?}");
            assert!(held.finalize(Some("<say>Hi.</say>")).is_some(), "{held:?}");
        }
        for open in [Person, Asked] {
            assert!(!open.held(), "{open:?}");
            assert!(open.finalize(Some("plain prose")).is_some(), "{open:?}");
        }
        assert_eq!(Person.contract(), None);
        assert_eq!(Asked.contract(), Some(super::SPOKEN_CONTRACT));
        // Any other agent's turn is a person's, whatever told it.
        assert_eq!(super::Origin::for_agent("ling", TaskEnd), Person);
        assert_eq!(super::Origin::for_agent("yinyue", TaskEnd), TaskEnd);
    }

    #[test]
    fn the_language_comes_from_what_the_person_wrote_never_from_the_kickoff() {
        use super::language_of_sample;
        assert_eq!(language_of_sample("帮我看看这个文件").as_deref(), Some("zh-Hans"));
        // English, or nothing to read: the UI language stands.
        assert_eq!(language_of_sample("please look at this file"), None);
        assert_eq!(language_of_sample(""), None);
    }

    #[test]
    fn the_contract_is_one_short_sentence_said_once() {
        let wrapped = with_contract("k".into(), Origin::Herald);
        assert_eq!(wrapped.matches("<say>").count(), 1, "{wrapped}");
        assert!(!wrapped.contains("SILENT") && !wrapped.contains("blank line"), "{wrapped}");
    }

    #[test]
    fn a_bare_silent_is_silence() {
        assert_eq!(answer_line("SILENT"), None);
        assert_eq!(answer_line("silent."), None);
        assert_eq!(answer_line("  SILENT\n"), None);
    }

    #[test]
    fn notes_above_a_blank_line_are_discarded() {
        // The three shapes deepseek-v4-flash produced on 2026-09-08.
        let silent = "The user said just \"hi\" and ling has replied. That's a routine \
                      greeting, not something worth pinging Hanli about. Silence.\n\nSILENT";
        assert_eq!(answer_line(silent), None);

        let spoken = "They asked something real: checking on Marine Thinking's IPO progress. \
                      Hanli is still away, but this is worth knowing.\n\n\
                      \"Ling's looked into the Marine Thinking IPO — the answer's waiting for you.\"";
        assert_eq!(
            answer_line(spoken).as_deref(),
            Some("Ling's looked into the Marine Thinking IPO — the answer's waiting for you.")
        );

        // The marker glued to the last sentence still means silence.
        assert_eq!(answer_line("Routine greeting. Silence. SILENT"), None);
    }

    #[test]
    fn a_plain_line_is_spoken_as_is() {
        assert_eq!(
            answer_line("Ling's reply is ready, Hanli.").as_deref(),
            Some("Ling's reply is ready, Hanli.")
        );
        // A natural sentence that merely ends in the word is not the marker.
        assert_eq!(
            answer_line("The room has gone silent.").as_deref(),
            Some("The room has gone silent.")
        );
        // A line broken across two lines is one line.
        assert_eq!(
            answer_line("Ling's reply is ready —\nthe IPO question.").as_deref(),
            Some("Ling's reply is ready — the IPO question.")
        );
    }
}

/// One place decides what she says. A publisher takes only a [`Line`], a
/// `Line` is made only in this file, and what is read of a turn's reply is
/// read through its [`Turned`] — whose origin is the turn's own. The scans
/// read production code only (`lock_scan`), literals and comments aside, and
/// fail loudly on source they cannot read.
///
/// What these are: tripwires for the plain forms, not a proof. A renamed
/// variable holding an origin, a write inside an allow-listed function, or a
/// helper that writes for a name passed in would slip past them. The real
/// barrier is the structure: private constructors (`Line`, `Turned`) and the
/// typed origin that decides held-ness, contract and finalizer in one place.
#[cfg(test)]
mod one_decision {
    use crate::server::lock_scan::production_code;

    /// Each file's count of `needle` in production code under `dirs`, against
    /// the files it may be in.
    fn check(dirs: &[&str], needle: &str, listed: &[(&str, usize)]) {
        for (rel, code) in production_code(dirs) {
            let found = code.matches(needle).count();
            let want = listed.iter().find(|(f, _)| *f == rel).map_or(0, |e| e.1);
            assert_eq!(found, want, "{rel}: `{needle}` — what she says is decided in one place; look at it, then list it");
        }
    }

    const ALL: &[&str] = &["src", "crates/linggen-engine/src"];

    #[test]
    fn a_line_of_hers_is_made_only_by_the_finalizer_or_the_answer_path() {
        // Built from parts, so this test's own text is no match.
        let make = format!("{}{}", "Line::", "new(");
        check(ALL, &make, &[("src/server/resident/spoken.rs", 4)]);
        let literal = format!("{}{}", "Line { ", "text");
        check(ALL, &literal, &[]);
        let answer = format!("{}{}", "Answer(", "Line");
        check(ALL, &answer, &[("src/server/resident/spoken.rs", 3)]);
    }

    #[test]
    fn the_envelope_is_read_in_one_place() {
        let open = format!("{}{}", "\"<", "say>\"");
        check(ALL, &open, &[("src/server/resident/spoken.rs", 1)]);
        let fin = format!("{}{}", "finalize_", "event(");
        check(ALL, &fin, &[("src/server/resident/spoken.rs", 2)]);
        let ans = format!("{}{}", "finalize_", "answer(");
        check(ALL, &ans, &[("src/server/resident/spoken.rs", 2)]);
    }

    /// A reply is read through the turn's own origin: `Turned::of` is handed
    /// a value (`origin`, `ctx.origin`), never a variant named at the call
    /// site, so no reader can pick another kind of turn's finalizer. The
    /// finalizers themselves are private to this file (the compiler's part).
    #[test]
    fn a_reply_is_read_with_the_turns_own_origin_never_a_named_variant() {
        for (rel, code) in production_code(ALL) {
            let mut rest = code.as_str();
            let call = format!("{}{}", "Turned::", "of(");
            while let Some(at) = rest.find(&call) {
                let args: String = rest[at..].chars().take_while(|c| *c != ')').collect();
                assert!(!args.contains("Origin::"), "{rel}: `{args}` names an origin where a turn's reply is read");
                rest = &rest[at + call.len()..];
            }
        }
        let fin = format!("{}{}", ".fin", "alize(");
        check(&["src/server/resident"], &fin, &[("src/server/resident/spoken.rs", 1), ("src/server/resident/triggers.rs", 1), ("src/server/resident/herald.rs", 1)]);
    }

    #[test]
    fn nothing_but_the_finalizers_output_is_published_as_a_row_of_hers() {
        let row = format!("{}{}", "fn keep_", "row(");
        check(ALL, &row, &[("src/server/resident/spoken.rs", 1)]);
        // The function's name: any variables, qualified or not.
        let publish = format!("{}{}", "publi", "sh(");
        check(
            &["src/server"],
            &publish,
            &[
                ("src/server/resident/spoken.rs", 2),
                ("src/server/resident/triggers.rs", 2),
                ("src/server/mac_tools.rs", 2),
                ("src/server/met_watch.rs", 2),
                ("src/server/api/topic.rs", 1),
            ],
        );
    }

    /// The chunks of `code` that are one function each, roughly: split where
    /// a `fn ` item begins.
    fn functions(code: &str) -> Vec<&str> {
        let mut starts: Vec<usize> = code.match_indices("fn ").map(|(i, _)| i).collect();
        starts.push(code.len());
        starts.windows(2).map(|w| &code[w[0]..w[1]]).collect()
    }

    /// A row or live message written as the companion is written by
    /// `keep_row` alone — anywhere under `src/server`: no other function that
    /// writes one names her.
    #[test]
    fn no_other_write_of_a_row_or_message_is_made_as_her_anywhere() {
        let writes = [
            format!("{}{}", "add_chat_", "message("),
            format!("{}{}", "ServerEvent::", "Message"),
            format!("{}{}", "persist_and_", "emit"),
            format!("{}{}", "persist_message_", "only"),
        ];
        let hers = ["COMPANION_AGENT_ID", "YINYUE_AGENT", "\"yinyue\"", "is_companion"];
        let mut found = Vec::new();
        for (rel, code) in production_code(&["src/server"]) {
            // The scan's own code and the test files (a `#[cfg(test)] mod x;`
            // is cut from its parent, but its file is read here).
            if rel == "src/server/lock_scan.rs" || rel.ends_with("/tests.rs") {
                continue;
            }
            for f in functions(&code) {
                if writes.iter().any(|w| f.contains(w.as_str())) && hers.iter().any(|h| f.contains(h)) {
                    let name: String = f.chars().skip(3).take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    found.push(format!("{rel}::{name}"));
                }
            }
        }
        found.sort();
        let allowed = [
            // her one writer
            "src/server/resident/spoken.rs::keep_row",
            // Rows of the PERSON'S message to her (a chat sent to her, her
            // kickoff kept as a hidden user row): written `from` the person
            // or the engine, never as her.
            "src/server/chat/handler.rs::dispatch_turn",
            "src/server/chat/handler.rs::start_turn",
            "src/server/resident/turn.rs::run_at",
        ];
        let others: Vec<&String> = found.iter().filter(|f| !allowed.contains(&f.as_str())).collect();
        assert_eq!(others, Vec::<&String>::new(), "a row or message written as her outside `keep_row` — look at it, then list it");
        assert_eq!(found.iter().filter(|f| allowed.contains(&f.as_str())).count(), allowed.len());
    }
}
