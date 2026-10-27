//! The sealed note: AES-256-GCM with the phone's own key, so Apple and the
//! relay carry bytes they cannot read. `nonce ‖ ciphertext ‖ tag`, base64 —
//! what CryptoKit's `AES.GCM.SealedBox(combined:)` opens on the phone.

use base64::{engine::general_purpose::STANDARD, Engine};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

/// Bound into every note; a note sealed for anything else will not open.
pub(crate) const AAD: &[u8] = b"linggen-push-v1";
/// The relay refuses more (APNs allows 4 KB for the whole payload).
pub(crate) const MAX_SEALED: usize = 3000;

/// The phone's key from its registration: base64 of exactly 32 bytes.
pub(crate) fn key_of(b64: &str) -> Option<[u8; 32]> {
    STANDARD.decode(b64.trim()).ok()?.try_into().ok()
}

/// Seal `plain` for the phone holding `key`.
pub(crate) fn seal(key: &[u8; 32], plain: &[u8]) -> Result<String, String> {
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| "no randomness".to_string())?;
    let sealing = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|_| "bad key")?);
    let mut buf = plain.to_vec();
    sealing
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(AAD),
            &mut buf,
        )
        .map_err(|_| "seal failed".to_string())?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&buf);
    let sealed = STANDARD.encode(out);
    if sealed.len() > MAX_SEALED {
        return Err(format!("note too long ({} > {MAX_SEALED})", sealed.len()));
    }
    Ok(sealed)
}

#[cfg(test)]
pub(crate) fn open(key: &[u8; 32], sealed: &str) -> Option<Vec<u8>> {
    let mut raw = STANDARD.decode(sealed).ok()?;
    let rest = raw.split_off(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(&raw).ok()?;
    let opening = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
    let mut rest = rest;
    let plain = opening
        .open_in_place(nonce, Aad::from(AAD), &mut rest)
        .ok()?;
    Some(plain.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_opens_with_its_key_and_no_other() {
        let key = [7u8; 32];
        let sealed = seal(&key, "{\"title\":\"早\"}".as_bytes()).expect("sealed");
        assert_eq!(
            open(&key, &sealed).as_deref(),
            Some("{\"title\":\"早\"}".as_bytes())
        );
        assert!(open(&[8u8; 32], &sealed).is_none());
        // nonce (12) + text + tag (16)
        assert_eq!(STANDARD.decode(&sealed).unwrap().len(), 12 + 15 + 16);
    }

    #[test]
    fn two_seals_of_one_note_differ() {
        let key = [1u8; 32];
        assert_ne!(seal(&key, b"x").unwrap(), seal(&key, b"x").unwrap());
    }

    #[test]
    fn a_key_is_exactly_32_bytes() {
        assert!(key_of(&STANDARD.encode([0u8; 32])).is_some());
        assert!(key_of(&STANDARD.encode([0u8; 16])).is_none());
        assert!(key_of("not base64!").is_none());
    }

    #[test]
    fn a_note_too_long_for_apns_is_refused() {
        assert!(seal(&[0u8; 32], &vec![b'a'; 4000]).is_err());
    }
}
