//! Her labels on a problem's offers — her words on each button, never code's.
//!
//! A kickoff that lists numbered offers asks her to name one or two of them
//! with a marker in her reply, `[offer 1: Free up space]`. The markers are
//! read here and taken out before anything is spoken ([`strip`], which
//! `spoken_line` applies), so her voice never says a label. The web UI strips
//! the same pattern from her thread rows (`ui/src/lib/petOffers.mts`, locked
//! by a test).

use crate::perception::offer::{Offer, Shown};

/// What the kickoff asks for when there are offers to label.
pub(super) const CONTRACT: &str = "Give one or two of the listed offers a label in your own words, \
    about three words each, as the person would say it to you — write each as [offer N: label] \
    at the end of your line, N being its number. Each becomes a button that runs that offer when \
    they tap it; nothing runs otherwise. The labels are never spoken.";

const OPEN: &str = "[offer ";
const LABEL_MAX: usize = 40;

/// The reply without its markers, and the labels it gave, by offer number.
pub(crate) fn take(reply: &str) -> (String, Vec<(usize, String)>) {
    let mut rest = reply.to_string();
    let mut labels = Vec::new();
    while let Some(start) = rest.find(OPEN) {
        let Some(len) = rest[start..].find(']') else {
            break;
        };
        let inner = &rest[start + OPEN.len()..start + len];
        if let Some((n, label)) = inner.split_once(':') {
            let label = label.trim();
            if let Ok(n) = n.trim().parse::<usize>() {
                if !label.is_empty() && label.chars().count() <= LABEL_MAX {
                    labels.push((n, label.to_string()));
                }
            }
        }
        rest.replace_range(start..start + len + 1, "");
    }
    let rest = rest
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    (rest, labels)
}

/// The reply as it may be spoken: every marker taken out.
pub(crate) fn strip(reply: &str) -> String {
    take(reply).0
}

/// Her labels bound to the offers they name — at most two, in her order; a
/// label for no listed offer, or a second for the same one, is dropped.
pub(super) fn bind(offers: &[Offer], labels: &[(usize, String)]) -> Vec<Shown> {
    let mut out: Vec<Shown> = Vec::new();
    let mut used = Vec::new();
    for (n, label) in labels {
        let Some(offer) = n.checked_sub(1).and_then(|i| offers.get(i)) else {
            continue;
        };
        if used.contains(n) {
            continue;
        }
        used.push(*n);
        out.push(Shown {
            label: label.clone(),
            offer: offer.clone(),
        });
        if out.len() == 2 {
            break;
        }
    }
    out
}

/// What a reply shows for a problem's offers, and whether the line is
/// unlabelled — offers to show and none named. Such a line is said without
/// buttons (`triggers::wake_and_offer`): never silent comes first.
pub(super) fn shown(offers: Option<&[Offer]>, reply: Option<&str>) -> (Vec<Shown>, bool) {
    let (Some(offers), Some(reply)) = (offers, reply) else {
        return (Vec::new(), false);
    };
    let shown = bind(offers, &take(reply).1);
    let unlabelled = !offers.is_empty() && shown.is_empty();
    (shown, unlabelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offers() -> Vec<Offer> {
        vec![
            Offer::Open { skill: "shifu".into(), view: Some("files".into()) },
            Offer::Tool {
                skill: "shifu".into(),
                tool: "ScanDisk".into(),
                args: serde_json::Value::Null,
                tier: "read".into(),
            },
        ]
    }

    #[test]
    fn her_labels_are_read_and_never_spoken() {
        let reply = "Your Mac is nearly full. [offer 2: Look what's big] [offer 1: Open Files]";
        let (rest, labels) = take(reply);
        assert_eq!(rest, "Your Mac is nearly full.");
        assert_eq!(labels, vec![(2, "Look what's big".into()), (1, "Open Files".into())]);
        let shown = bind(&offers(), &labels);
        assert_eq!(shown.len(), 2);
        assert_eq!(shown[0].label, "Look what's big");
        assert!(matches!(shown[0].offer, Offer::Tool { .. }));
        // What reaches her voice carries no marker.
        let spoken = super::super::spoken::spoken_line(reply).unwrap();
        assert_eq!(spoken, "Your Mac is nearly full.");
    }

    #[test]
    fn a_label_for_no_listed_offer_is_dropped() {
        let (_, labels) = take("x [offer 3: Nope] [offer 1: Yes] [offer 1: Again] [offer one: Bad]");
        let shown = bind(&offers(), &labels);
        assert_eq!(shown.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["Yes"]);
    }

    #[test]
    fn a_line_that_labels_no_offer_is_known_as_unlabelled() {
        let o = offers();
        assert!(shown(Some(&o), Some("Your Mac is nearly full.")).1);
        assert!(!shown(Some(&o), Some("Full. [offer 1: Open Files]")).1);
        assert!(!shown(Some(&[]), Some("Nothing can be done here.")).1, "a none-remedy line is said bare");
        assert!(!shown(None, Some("A quiet remark.")).1);
    }

    #[test]
    fn the_web_ui_strips_the_same_marker() {
        let ui = include_str!("../../../ui/src/lib/petOffers.mts");
        assert!(ui.contains("[offer "), "petOffers.mts must strip the [offer N: label] marker");
    }
}
