//! Shared complete minisign verification for the updater and release verifier.
use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};

pub fn verify(
    bytes: &[u8],
    encoded_key: &str,
    encoded_signature: &str,
    version: &str,
) -> Result<(), String> {
    let decode = |value: &str| -> Result<String, String> {
        String::from_utf8(
            STANDARD
                .decode(value.trim())
                .map_err(|_| "invalid base64")?,
        )
        .map_err(|_| "invalid UTF-8".into())
    };
    let key = PublicKey::decode(&decode(encoded_key)?).map_err(|_| "invalid public key")?;
    let text = decode(encoded_signature)?;
    let signature = Signature::decode(&text).map_err(|_| "invalid signature")?;
    // minisign-verify always checks the global signature; false rejects legacy algorithms.
    key.verify(bytes, &signature, false)
        .map_err(|_| "signature verification failed")?;
    // This comment is authenticated by the complete verification above.
    let comment = text
        .lines()
        .find_map(|line| line.strip_prefix("trusted comment: "))
        .ok_or("missing signed version")?;
    let versions: Vec<_> = comment
        .split_whitespace()
        .filter_map(|field| field.strip_prefix("version:"))
        .collect();
    if versions != [version] {
        return Err("signed version mismatch".into());
    }
    Ok(())
}
