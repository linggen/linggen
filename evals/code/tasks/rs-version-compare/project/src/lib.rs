//! Compare dotted version strings such as "1.10.2". A leading "v" is allowed
//! ("v1.10.2"), as release tags are written that way.

#[cfg(test)]
mod tests;

/// True when version `a` is newer than version `b`.
pub fn newer(a: &str, b: &str) -> bool {
    key(a) > key(b)
}

fn key(v: &str) -> String {
    v.trim_start_matches('v').to_string()
}
