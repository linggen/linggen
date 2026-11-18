//! The contract number: what this Mac and a phone each say they speak, so an
//! out-of-step pair is told so plainly instead of a feature failing silently.
//!
//! The Mac sends both numbers in `/api/health` and in the greeting it
//! answers `identify` with; a phone sends its `contract` in `identify` (none
//! is contract 0). A side below the other's `oldest_peer` is out of step: the
//! phone shows one "update" fact, and the Mac lists the device as needing an
//! update and gives it chat at most (`person::standing_of`).
//!
//! The phone keeps the same two numbers in `lib/services/rtc/contract.dart`;
//! a test there reads this file and fails when they differ.

/// The contract this Mac speaks.
pub(crate) const CONTRACT: u32 = 1;

/// The oldest phone contract this Mac still works with.
pub(crate) const OLDEST_PEER: u32 = 1;

/// The contract an `identify` frame says its phone speaks; none is 0.
pub(crate) fn told(identify: &serde_json::Value) -> u32 {
    identify
        .get("contract")
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

/// Whether a phone speaking `contract` is too old for this Mac.
pub(crate) fn behind(contract: u32) -> bool {
    contract < OLDEST_PEER
}

/// The two numbers, as `/api/health` and the greeting carry them.
pub(crate) fn numbers() -> serde_json::Value {
    serde_json::json!({ "contract": CONTRACT, "oldest_peer": OLDEST_PEER })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A phone that says nothing is contract 0, told to update; this
    /// release's own number passes.
    #[test]
    fn a_phone_that_names_no_contract_is_behind() {
        assert_eq!(told(&serde_json::json!({"type": "identify"})), 0);
        assert!(behind(told(&serde_json::json!({"type": "identify"}))));
        assert_eq!(told(&serde_json::json!({"contract": CONTRACT})), CONTRACT);
        assert!(!behind(CONTRACT));
        assert_eq!(told(&serde_json::json!({"contract": "1"})), 0);
        assert_eq!(
            numbers(),
            serde_json::json!({"contract": 1, "oldest_peer": 1})
        );
    }
}
