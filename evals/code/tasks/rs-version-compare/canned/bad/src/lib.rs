//! Compare dotted version strings such as "1.10.2".

#[cfg(test)]
mod tests;

/// True when version `a` is newer than version `b`.
pub fn newer(a: &str, b: &str) -> bool {
    key(a) > key(b)
}

fn key(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}
