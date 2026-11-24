//! A family moves only to a model that has answered. When the backend lists
//! a newer visible model of a built-in family, it is tried once with the
//! smallest request a turn can make; only an answer makes it the family's
//! model. Until then, and after a refusal, the previous one stays, and the
//! try is repeated at the next refresh — never on a turn. A model the
//! account's plan does not include is never listed, so never tried.

use super::{slug_family, Snapshot};
use async_trait::async_trait;

/// Whether a model answers. Tests pass their own.
#[async_trait]
pub trait Prober: Send + Sync {
    async fn answers(&self, slug: &str) -> bool;
}

/// How long a try may take before it counts as no answer.
const PROBE_SECS: u64 = 60;

/// The real try: one short message down the same request path a turn uses,
/// on a bare client (no health record, no fallback), stopping at the first
/// good chunk.
pub struct ChatgptProber {
    client: crate::provider::openai::OpenAiClient,
}

impl ChatgptProber {
    pub fn new(client: crate::provider::openai::OpenAiClient) -> Self {
        Self { client }
    }

    async fn first_chunk(&self, slug: &str) -> anyhow::Result<bool> {
        use futures_util::StreamExt;
        let ask = [crate::message::ChatMessage::new("user", "Reply with OK.")];
        let stream = self.client.chat_text_stream(slug, &ask, None, None).await?;
        let mut stream = Box::pin(stream);
        match stream.next().await {
            Some(Ok(_)) => Ok(true),
            Some(Err(e)) => Err(e),
            None => Ok(false),
        }
    }
}

#[async_trait]
impl Prober for ChatgptProber {
    async fn answers(&self, slug: &str) -> bool {
        let timeout = std::time::Duration::from_secs(PROBE_SECS);
        match tokio::time::timeout(timeout, self.first_chunk(slug)).await {
            Ok(Ok(answered)) => answered,
            Ok(Err(e)) => {
                tracing::info!("chatgpt catalog: {slug} did not answer: {e}");
                false
            }
            Err(_) => {
                tracing::info!("chatgpt catalog: {slug} did not answer within {PROBE_SECS}s");
                false
            }
        }
    }
}

/// The model [family] would move to: the newest (lowest priority) listed
/// model of the family, when it is not the current one and the current one
/// is not listed ahead of it.
pub fn candidate(snap: &Snapshot, family: &str) -> Option<String> {
    let fam = snap.family(family)?;
    let newest = snap
        .listed
        .iter()
        .filter(|m| slug_family(&m.slug) == Some(family))
        .min_by_key(|m| m.priority)?;
    if newest.slug == fam.current {
        return None;
    }
    let current = snap.listed.iter().find(|m| m.slug == fam.current);
    if current.is_some_and(|c| c.priority <= newest.priority) {
        return None;
    }
    Some(newest.slug.clone())
}

/// Try each family's candidate and move the ones that answered. True when a
/// family moved.
pub async fn promote(snap: &mut Snapshot, prober: &dyn Prober, now: u64) -> bool {
    let names: Vec<String> = snap.families.iter().map(|f| f.name.clone()).collect();
    // A refusal is remembered only while the backend still lists the model.
    let listed: Vec<String> = snap.listed.iter().map(|m| m.slug.clone()).collect();
    snap.refused.retain(|slug, _| listed.contains(slug));
    let mut moved = false;
    for name in names {
        let Some(next) = candidate(snap, &name) else {
            continue;
        };
        if !prober.answers(&next).await {
            snap.refused.insert(next, now);
            continue;
        }
        snap.refused.remove(&next);
        move_family(snap, &name, next, now);
        moved = true;
    }
    moved
}

/// [name] now runs on [next]; what it ran on becomes the one "stay" goes
/// back to. A new move starts with the person following it.
fn move_family(snap: &mut Snapshot, name: &str, next: String, now: u64) {
    let Some(fam) = snap.family_mut(name) else {
        return;
    };
    let was = std::mem::replace(&mut fam.current, next);
    if !fam.history.contains(&was) {
        fam.history.push(was.clone());
    }
    fam.previous = Some(was);
    fam.moved_at = Some(now);
    fam.stay = false;
    tracing::info!("chatgpt catalog: {name} moved to {}", fam.current);
}

#[cfg(test)]
mod tests {
    use super::super::{tests::measured_list, Listed};
    use super::*;
    use std::sync::Mutex;

    /// Answers for the slugs it is told, and remembers every try.
    struct Fake {
        answering: Vec<&'static str>,
        tried: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(answering: &[&'static str]) -> Self {
            Self { answering: answering.to_vec(), tried: Mutex::new(vec![]) }
        }
        fn tried(&self) -> Vec<String> {
            self.tried.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Prober for Fake {
        async fn answers(&self, slug: &str) -> bool {
            self.tried.lock().unwrap().push(slug.to_string());
            self.answering.contains(&slug)
        }
    }

    /// The measured list plus a synthetic next Sol, listed first.
    fn with_next_sol() -> Snapshot {
        let mut s = Snapshot::seed();
        s.listed = measured_list();
        s.listed.insert(0, Listed { slug: "gpt-7-sol".into(), ..Listed::default() });
        s
    }

    #[tokio::test]
    async fn the_measured_list_tries_nothing_and_moves_nothing() {
        let mut s = Snapshot::seed();
        s.listed = measured_list();
        let fake = Fake::new(&[]);
        assert!(!promote(&mut s, &fake, 5).await);
        assert!(fake.tried().is_empty());
        assert_eq!(s.builtin_ids(), Snapshot::seed().builtin_ids());
    }

    #[tokio::test]
    async fn a_newer_model_is_not_current_until_it_has_answered() {
        let mut s = with_next_sol();
        let refusing = Fake::new(&[]);
        assert!(!promote(&mut s, &refusing, 5).await);
        assert_eq!(refusing.tried(), vec!["gpt-7-sol"]);
        assert_eq!(s.builtin_ids()[0], "gpt-6.2-sol");
        assert_eq!(s.refused.get("gpt-7-sol"), Some(&5));
        // A pin to it still runs on it — it is listed — but a default does not.
        assert_eq!(s.follow("gpt-7-sol").as_deref(), Some("gpt-6.2-sol"));

        // The next refresh tries again; this time it answers.
        let answering = Fake::new(&["gpt-7-sol"]);
        assert!(promote(&mut s, &answering, 9).await);
        assert_eq!(s.builtin_ids(), vec!["gpt-7-sol", "gpt-6.1-terra", "gpt-6-luna"]);
        let sol = s.family("sol").unwrap();
        assert_eq!(sol.previous.as_deref(), Some("gpt-6.2-sol"));
        assert_eq!(sol.moved_at, Some(9));
        assert!(s.refused.is_empty());
        assert_eq!(s.follow("gpt-6.2-sol").as_deref(), Some("gpt-7-sol"));
        assert_eq!(s.follow("gpt-5.6-sol").as_deref(), Some("gpt-7-sol"));

        // Moved, nothing more to try.
        let again = Fake::new(&["gpt-7-sol"]);
        assert!(!promote(&mut s, &again, 12).await);
        assert!(again.tried().is_empty());
    }

    #[tokio::test]
    async fn astra_is_not_a_built_in_family_and_is_never_tried() {
        let mut s = Snapshot::seed();
        s.listed = measured_list();
        s.listed.insert(0, Listed { slug: "gpt-7-astra".into(), ..Listed::default() });
        let fake = Fake::new(&["gpt-7-astra"]);
        assert!(!promote(&mut s, &fake, 5).await);
        assert!(fake.tried().is_empty());
    }

    #[tokio::test]
    async fn a_dropped_current_model_moves_only_to_one_that_answers() {
        let mut s = Snapshot::seed();
        s.listed = measured_list();
        s.listed.retain(|m| m.slug != "gpt-6-luna");
        assert_eq!(candidate(&s, "luna").as_deref(), Some("gpt-5.6-luna"));
        let refusing = Fake::new(&[]);
        promote(&mut s, &refusing, 5).await;
        assert_eq!(s.primary_id(), "gpt-6-luna");
    }
}
