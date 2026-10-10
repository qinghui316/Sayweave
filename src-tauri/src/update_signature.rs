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
    // Use only the comment authenticated by the global signature. The first
    // signature-file line is untrusted and may contain any text.
    let comment = signature.trusted_comment();
    let versions: Vec<_> = comment
        .split_whitespace()
        .filter_map(|field| field.strip_prefix("version:"))
        .collect();
    if versions != [version] {
        return Err("signed version mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Static synthetic Ed25519/minisign fixture for the bytes "test".
    const KEY: &str = "untrusted comment: regression test public key\nRWQBAgMEBQYHCOpKbGPinFIKvvVQexMuxfmVR3auvr57kkIe6mkURtIs";
    const SIGNATURE: &str = "untrusted comment: regression fixture\nRUQBAgMEBQYHCBh0od4foi9vkvb4iisNrDD8+mHIhwhSI7T4vRrHtGY5mLCnK5qqaRTd29Unl3vY9jMAiPhE5/iowuCEI5mb8A0=\ntrusted comment: version:0.1.7\nDd+fFY1XNBinAAMYSwCFwhKWeeBWYd3IDL2X5ougxuKHWFwb/W2R+7HBRhNU17QMtDgPmqpEMyCeGIBnYjIBBw==";

    #[test]
    fn accepts_only_the_authenticated_version() {
        assert!(
            verify(
                b"test",
                &STANDARD.encode(KEY),
                &STANDARD.encode(SIGNATURE),
                "0.1.7"
            )
            .is_ok()
        );
        assert!(
            verify(
                b"test",
                &STANDARD.encode(KEY),
                &STANDARD.encode(SIGNATURE),
                "0.1.8"
            )
            .is_err()
        );
        assert!(
            verify(
                b"different",
                &STANDARD.encode(KEY),
                &STANDARD.encode(SIGNATURE),
                "0.1.7"
            )
            .is_err()
        );
    }

    #[test]
    fn untrusted_annotation_cannot_override_the_signed_version() {
        let changed = SIGNATURE.replacen(
            "untrusted comment: regression fixture",
            "trusted comment: version:0.1.8",
            1,
        );
        assert_eq!(
            verify(
                b"test",
                &STANDARD.encode(KEY),
                &STANDARD.encode(&changed),
                "0.1.8"
            ),
            Err("signed version mismatch".into())
        );
        assert!(
            verify(
                b"test",
                &STANDARD.encode(KEY),
                &STANDARD.encode(changed),
                "0.1.7"
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_modified_authenticated_annotation() {
        let changed = SIGNATURE.replace("version:0.1.7", "version:0.1.8");
        assert_eq!(
            verify(
                b"test",
                &STANDARD.encode(KEY),
                &STANDARD.encode(changed),
                "0.1.8"
            ),
            Err("signature verification failed".into())
        );
    }

    #[test]
    fn rejects_missing_and_duplicate_authenticated_versions() {
        for (comment, global) in [
            (
                "timestamp:1 file:test",
                "CExukC5kw0Nzb4nd8PvrE6iXQd2TVePm9TVVkDshEXrIFSUCDJafyReFIXuXSVRo2UjTm5Jlv6xDIRrTvjzLBA==",
            ),
            (
                "version:0.1.7 version:0.1.7",
                "l34XJB2lgxD0fANFNTOIuOlxQk2XLFkpHxUqlFDcmXVTWRqwImEpkHSBKKUEexzM/lx7W7ArZt4G+3o5QbnsAA==",
            ),
        ] {
            let signed = format!(
                "trusted comment: version:0.1.7\n{}\ntrusted comment: {comment}\n{global}",
                SIGNATURE.lines().nth(1).unwrap()
            );
            assert_eq!(
                verify(
                    b"test",
                    &STANDARD.encode(KEY),
                    &STANDARD.encode(signed),
                    "0.1.7"
                ),
                Err("signed version mismatch".into())
            );
        }
    }
}
