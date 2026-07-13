//! OAuth2 flows for Stalwart Mail Server.
//!
//! Supports:
//! - Well-known metadata discovery (RFC 8414)
//! - Authorization Code + PKCE (browser callback) — see [`browser`]
//! - Device Authorization Grant (polling) — see [`device`]

mod browser;
mod device;
mod pkce;
mod types;

pub use types::{DeviceAuthResponse, OAuthMetadata, OAuthTokenResponse, TokenStore};

use thiserror::Error;
use tracing::{debug, info};

#[derive(Debug, Error)]
pub enum OAuthError {
    #[error("OAuth discovery failed: {0}")]
    Discovery(String),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Token exchange failed: {0}")]
    TokenExchange(String),
    #[error("Device flow expired or denied")]
    DeviceFlowExpired,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("URL parse error: {0}")]
    Url(#[from] url::ParseError),
}

/// OAuth client for a Stalwart server.
pub struct StalwartOAuth {
    pub(crate) http: reqwest::Client,
    pub(crate) server_url: String,
    pub(crate) client_id: String,
}

impl StalwartOAuth {
    pub fn new(server_url: &str, client_id: &str) -> Result<Self, OAuthError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            server_url: server_url.trim_end_matches('/').to_string(),
            client_id: client_id.to_string(),
        })
    }

    /// Discover OAuth metadata from the server's well-known endpoints.
    pub async fn discover(&self) -> Result<OAuthMetadata, OAuthError> {
        let urls = [
            format!("{}/.well-known/oauth-authorization-server", self.server_url),
            format!("{}/.well-known/openid-configuration", self.server_url),
        ];

        for url in &urls {
            debug!("Trying OAuth discovery at {}", url);
            match self.http.get(url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let meta: OAuthMetadata = resp.json().await.map_err(|e| {
                        OAuthError::Discovery(format!("failed to parse metadata: {e}"))
                    })?;
                    info!("OAuth metadata discovered from {}", url);
                    return Ok(meta);
                }
                Ok(resp) => {
                    debug!("Discovery {} returned {}", url, resp.status());
                }
                Err(e) => {
                    debug!("Discovery {} failed: {}", url, e);
                }
            }
        }

        Err(OAuthError::Discovery(format!(
            "no OAuth metadata found at {}",
            self.server_url
        )))
    }

    /// Exchange a refresh token for a new access token.
    pub async fn refresh_token(
        &self,
        refresh_token: &str,
    ) -> Result<OAuthTokenResponse, OAuthError> {
        let meta = self.discover().await?;

        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &self.client_id),
        ];

        let resp = self
            .http
            .post(&meta.token_endpoint)
            .form(&params)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OAuthError::TokenExchange(format!(
                "refresh failed {status}: {body}"
            )));
        }

        Ok(resp.json().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalwart_oauth_new_valid_url() {
        let oauth = StalwartOAuth::new("https://mail.example.com", "herald");
        assert!(oauth.is_ok(), "new() should succeed with a valid URL");
        let oauth = oauth.unwrap();
        assert_eq!(oauth.server_url, "https://mail.example.com");
        assert_eq!(oauth.client_id, "herald");
    }

    #[test]
    fn stalwart_oauth_new_strips_trailing_slash() {
        let oauth = StalwartOAuth::new("https://mail.example.com/", "herald").unwrap();
        assert_eq!(oauth.server_url, "https://mail.example.com");
    }
}
