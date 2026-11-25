//! Who the list could be trusted from: the one place the public key is written.
//!
//! Reading a list does not require a signature today (the owner's decision:
//! the list travels over HTTPS). The signing code is kept so a later decision
//! to require one is a single call at `bundle::decode`'s callers. The private
//! half is the owner's and lives nowhere in this repository.

/// A THROWAWAY test key (its private half was made for slice 1 and is not
/// kept). Replace it with the owner's PUBLIC key before a signature is ever
/// required (`node scripts/print-model-catalog-key.mjs <key.pem> --rust` in
/// the site repository prints the array).
#[allow(dead_code)]
pub(super) const OWNER_PUBLIC_KEY: [u8; 32] = [
    0x99, 0x67, 0xca, 0x69, 0x7e, 0xca, 0x59, 0x47,
    0x7f, 0xa7, 0x9f, 0x6c, 0x1f, 0x51, 0x69, 0x6d,
    0x74, 0x8a, 0xc9, 0x1e, 0xfe, 0xb8, 0x08, 0xb5,
    0x16, 0xa4, 0x5b, 0xb9, 0xf4, 0x7b, 0xd2, 0x3b,
];
