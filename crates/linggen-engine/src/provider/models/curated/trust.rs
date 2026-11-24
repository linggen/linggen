//! Who the list is trusted from: the one place the public key is written.
//!
//! The private half is the owner's and lives nowhere in this repository.
//! Release procedure: put the owner's PUBLIC key in `OWNER_PUBLIC_KEY`
//! (`node scripts/print-model-catalog-key.mjs <key.pem> --rust` in the site
//! repository prints the array), set `OWNER_KEY_IS_PLACEHOLDER` to false,
//! re-sign the bundle with that key (`scripts/build-model-catalog.mjs
//! --sign-key`) and copy the site's `catalog/owner-public-key.hex` to match.
//! A test in the site repository compares this array with that file.

/// A THROWAWAY test key (its private half was made for slice 1 and is not
/// kept). It is a placeholder so the shipped bundle verifies; replace it
/// before any refresh is turned on.
pub(super) const OWNER_PUBLIC_KEY: [u8; 32] = [
    0x61, 0xfd, 0x53, 0xc4, 0x7d, 0x3d, 0x0d, 0x80,
    0x0c, 0x50, 0x0d, 0x04, 0x44, 0xf6, 0x5d, 0xa9,
    0x8c, 0x92, 0xd7, 0x24, 0xac, 0x7c, 0x07, 0x73,
    0xfe, 0x63, 0x03, 0x95, 0x54, 0x8d, 0x43, 0x43,
];

/// True while [`OWNER_PUBLIC_KEY`] is the throwaway one. `store` refuses, at
/// compile time, to turn refresh on while this is true.
pub(super) const OWNER_KEY_IS_PLACEHOLDER: bool = true;
