//! PKCE helpers: code verifier, S256 challenge, and state generation.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::Rng;
use sha2::{Digest, Sha256};

use super::StalwartOAuth;

impl StalwartOAuth {
    pub(crate) fn generate_verifier() -> String {
        let mut buf = [0u8; 32];
        rand::thread_rng().fill(&mut buf);
        URL_SAFE_NO_PAD.encode(buf)
    }

    pub(crate) fn generate_challenge(verifier: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        URL_SAFE_NO_PAD.encode(hasher.finalize())
    }

    pub(crate) fn generate_state() -> String {
        let mut buf = [0u8; 16];
        rand::thread_rng().fill(&mut buf);
        URL_SAFE_NO_PAD.encode(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_verifier_produces_43_char_base64url() {
        let verifier = StalwartOAuth::generate_verifier();
        assert_eq!(verifier.len(), 43, "32 bytes base64url-no-pad = 43 chars");
        // Verify it's valid base64url (no +, /, or = characters)
        assert!(
            verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "verifier must be base64url: {verifier}"
        );
    }

    #[test]
    fn generate_challenge_produces_valid_sha256_base64url() {
        let verifier = StalwartOAuth::generate_verifier();
        let challenge = StalwartOAuth::generate_challenge(&verifier);
        // SHA-256 = 32 bytes → base64url-no-pad = 43 chars
        assert_eq!(challenge.len(), 43, "SHA-256 base64url-no-pad = 43 chars");
        assert!(
            challenge
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "challenge must be base64url: {challenge}"
        );

        // Verify it matches a manual SHA-256 computation
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(hasher.finalize());
        assert_eq!(challenge, expected);
    }

    #[test]
    fn generate_state_produces_22_char_base64url() {
        let state = StalwartOAuth::generate_state();
        assert_eq!(state.len(), 22, "16 bytes base64url-no-pad = 22 chars");
        assert!(
            state
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "state must be base64url: {state}"
        );
    }
}
