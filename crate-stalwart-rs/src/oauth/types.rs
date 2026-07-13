//! OAuth wire types (RFC 8414 metadata, token responses) and the token store.

use serde::{Deserialize, Serialize};

/// OAuth server metadata (RFC 8414).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthMetadata {
    pub issuer: Option<String>,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub revocation_endpoint: Option<String>,
    #[serde(default)]
    pub device_authorization_endpoint: Option<String>,
}

/// Token endpoint response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Device authorization endpoint response (RFC 8628).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    #[serde(default = "default_expires_in")]
    pub expires_in: u64,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_expires_in() -> u64 {
    600
}
fn default_interval() -> u64 {
    5
}

/// Cached tokens with expiry.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenStore {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
}

impl TokenStore {
    /// Treat tokens as expired if they have less than 60 seconds remaining.
    /// This avoids using a token that will expire mid-request.
    const EXPIRY_SKEW_SECS: u64 = 60;

    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(exp) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                now + Self::EXPIRY_SKEW_SECS >= exp
            }
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_store_is_expired_past() {
        let store = TokenStore {
            access_token: Some("tok".into()),
            refresh_token: None,
            expires_at: Some(0), // epoch = definitely in the past
        };
        assert!(store.is_expired(), "expires_at=0 should be expired");
    }

    #[test]
    fn token_store_is_expired_future() {
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600; // one hour from now
        let store = TokenStore {
            access_token: Some("tok".into()),
            refresh_token: None,
            expires_at: Some(future),
        };
        assert!(
            !store.is_expired(),
            "expires_at in the future should NOT be expired"
        );
    }

    #[test]
    fn token_store_is_expired_within_skew_window() {
        // Token expiring in 30 seconds (within the 60-second skew window) should be expired
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let store = TokenStore {
            access_token: Some("tok".into()),
            refresh_token: None,
            expires_at: Some(now + 30),
        };
        assert!(
            store.is_expired(),
            "token expiring in 30s (within 60s skew) should be treated as expired"
        );
    }

    #[test]
    fn token_store_is_not_expired_outside_skew_window() {
        // Token expiring in 120 seconds (well outside the 60-second skew window) should NOT be expired
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let store = TokenStore {
            access_token: Some("tok".into()),
            refresh_token: None,
            expires_at: Some(now + 120),
        };
        assert!(
            !store.is_expired(),
            "token expiring in 120s (outside 60s skew) should NOT be treated as expired"
        );
    }

    #[test]
    fn token_store_is_expired_none() {
        let store = TokenStore::default();
        assert!(
            store.is_expired(),
            "None expires_at should be treated as expired"
        );
    }
}
