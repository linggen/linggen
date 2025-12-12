use anyhow::{Context, Result};
use base64::Engine;
use std::fs::File;
use std::path::Path;

// Public key for signature verification (base64-encoded minisign public key text).
// This must match the key embedded in the desktop app's `tauri.conf.json`.
pub const LINGGEN_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE5NkVGNURDQzExNkE1OEEKUldTS3BSYkIzUFZ1R2YzVUVaQkZPQ0V1TXIzbzhPakhzaUM5SWo3dTVVeUxra3lhVmZ2MzhTVzMK";

/// Verify a file's signature using minisign.
///
/// `signature` must be the base64-encoded content of a minisign `.sig` file.
pub fn verify_signature(file_path: &Path, signature: &str, public_key: &str) -> Result<()> {
    use minisign::{PublicKey, PublicKeyBox, SignatureBox};

    // Decode the base64-encoded public key to get the minisign format
    let pubkey_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key)
        .context("Failed to decode public key from base64")?;

    // Parse the public key (minisign format: comment line + key line)
    let pubkey_str = String::from_utf8(pubkey_bytes).context("Public key is not valid UTF-8")?;

    // Parse the public key box and convert to public key
    let pk_box = PublicKeyBox::from_string(&pubkey_str)
        .context("Failed to parse public key - ensure it's in minisign format")?;

    // Convert PublicKeyBox to PublicKey
    // Try from_box first (for trusted keys), fallback to into_public_key (for untrusted)
    let pk = match PublicKey::from_box(pk_box.clone()) {
        Ok(pk) => pk,
        Err(_) => pk_box.into_public_key().context(
            "Failed to convert public key box to public key - key may be invalid or in wrong format",
        )?,
    };

    // Decode the base64 signature to get minisign signature format text.
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature)
        .context("Failed to decode signature from base64")?;
    let sig_str = String::from_utf8(sig_bytes).context("Signature is not valid UTF-8")?;
    let sig_box = SignatureBox::from_string(&sig_str).context("Failed to parse signature")?;

    let data_reader = File::open(file_path)
        .with_context(|| format!("Failed to open file: {}", file_path.display()))?;

    // Verify the signature.
    //
    // minisign::verify args are: (pk, sig, data_reader, quiet, output, allow_legacy)
    // `output=true` will copy the *entire file* to stdout (looks like garbage for .tar.gz),
    // so keep it disabled.
    minisign::verify(&pk, &sig_box, data_reader, true, false, false)
        .context("Signature verification failed")?;

    Ok(())
}
