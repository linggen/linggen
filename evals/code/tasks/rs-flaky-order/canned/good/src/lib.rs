//! Release notes from a list of entries, grouped by kind.

use std::collections::HashMap;

#[cfg(test)]
mod tests;

/// One line of the changelog: what kind of change it is, and what it says.
pub struct Entry {
    pub kind: String,
    pub text: String,
}

/// The kinds that appear in the notes, in the order their sections appear.
/// An entry of any other kind is left out.
const ORDER: [&str; 4] = ["Breaking", "Added", "Changed", "Fixed"];

/// The notes as text: one `## Kind` section for each kind that has entries,
/// sections in `ORDER`, entries in the order they were given, a blank line
/// between sections.
pub fn render(entries: &[Entry]) -> String {
    let mut groups: HashMap<&str, Vec<&str>> = HashMap::new();
    for entry in entries {
        groups
            .entry(entry.kind.as_str())
            .or_default()
            .push(entry.text.as_str());
    }
    let mut sections = Vec::new();
    for kind in ORDER {
        if let Some(texts) = groups.get(kind) {
            let mut section = format!("## {}\n", kind);
            for text in texts {
                section.push_str(&format!("- {}\n", text));
            }
            sections.push(section);
        }
    }
    sections.join("\n")
}
