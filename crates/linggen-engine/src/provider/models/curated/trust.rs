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
    0x61, 0xfd, 0x53, 0xc4, 0x7d, 0x3d, 0x0d, 0x80,
    0x0c, 0x50, 0x0d, 0x04, 0x44, 0xf6, 0x5d, 0xa9,
    0x8c, 0x92, 0xd7, 0x24, 0xac, 0x7c, 0x07, 0x73,
    0xfe, 0x63, 0x03, 0x95, 0x54, 0x8d, 0x43, 0x43,
];
