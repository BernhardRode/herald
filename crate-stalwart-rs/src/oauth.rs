//! OAuth2 flows for Stalwart Mail Server.
//!
//! Supports:
//! - Well-known metadata discovery
//! - Authorization Code + PKCE (browser callback)
//! - Device Authorization Grant (polling)

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpListener;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// OAuth Metadata (RFC 8414)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Token types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Token storage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenStore {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
}

impl TokenStore {
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(exp) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                now >= exp
            }
            None => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Main OAuth client
// ---------------------------------------------------------------------------

pub struct StalwartOAuth {
    http: reqwest::Client,
    server_url: String,
    client_id: String,
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

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // PKCE helpers
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Browser flow (Authorization Code + PKCE)
    // -----------------------------------------------------------------------

    /// Perform the full OAuth2 Authorization Code flow with PKCE.
    ///
    /// 1. Discovers OAuth metadata
    /// 2. Opens browser to authorization endpoint
    /// 3. Listens on localhost for the redirect callback
    /// 4. Exchanges the authorization code for tokens
    pub async fn browser_flow(&self) -> Result<OAuthTokenResponse, OAuthError> {
        let meta = self.discover().await?;

        let verifier = Self::generate_verifier();
        let challenge = Self::generate_challenge(&verifier);
        let state = Self::generate_state();

        // Bind a random port for the callback
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");

        // Build authorization URL
        let mut auth_url = url::Url::parse(&meta.authorization_endpoint)?;
        auth_url
            .query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);

        info!("Opening browser for OAuth login...");
        println!("Opening browser to: {}", auth_url);
        println!("If browser doesn't open, visit the URL above manually.");

        if open::that(auth_url.as_str()).is_err() {
            warn!("Failed to open browser automatically");
        }

        // Wait for callback
        let code = tokio::task::spawn_blocking(move || -> Result<String, OAuthError> {
            let (mut stream, _) = listener.accept()?;
            let mut buf = [0u8; 4096];
            let n = std::io::Read::read(&mut stream, &mut buf)?;
            let request = String::from_utf8_lossy(&buf[..n]);

            // Parse the GET request for the code and state
            let first_line = request.lines().next().unwrap_or("");
            let path = first_line.split_whitespace().nth(1).unwrap_or("");
            let url = url::Url::parse(&format!("http://localhost{path}"))
                .map_err(OAuthError::Url)?;

            let params: HashMap<String, String> = url.query_pairs().into_owned().collect();

            // Verify state
            let received_state = params.get("state").cloned().unwrap_or_default();
            if received_state != state {
                let response = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\n\r\n<html><body><h1>State mismatch!</h1></body></html>";
                stream.write_all(response.as_bytes())?;
                return Err(OAuthError::TokenExchange("state mismatch".into()));
            }

            let code = params
                .get("code")
                .ok_or_else(|| {
                    OAuthError::TokenExchange(
                        params
                            .get("error_description")
                            .or(params.get("error"))
                            .cloned()
                            .unwrap_or_else(|| "no code in callback".into()),
                    )
                })?
                .clone();

            let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h1>Login successful!</h1><p>You can close this tab and return to the terminal.</p></body></html>";
            stream.write_all(response.as_bytes())?;

            Ok(code)
        })
        .await
        .map_err(|e| OAuthError::TokenExchange(format!("callback task failed: {e}")))??;

        // Exchange code for tokens
        self.exchange_code(&code, &verifier, &redirect_uri, &meta.token_endpoint)
            .await
    }

    async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
        token_endpoint: &str,
    ) -> Result<OAuthTokenResponse, OAuthError> {
        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", &self.client_id),
            ("code_verifier", verifier),
        ];

        let resp = self.http.post(token_endpoint).form(&params).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OAuthError::TokenExchange(format!(
                "token endpoint returned {status}: {body}"
            )));
        }

        Ok(resp.json().await?)
    }

    // -----------------------------------------------------------------------
    // Device Authorization Grant (RFC 8628)
    // -----------------------------------------------------------------------

    /// Perform the OAuth2 Device Authorization Grant flow.
    ///
    /// 1. Discovers OAuth metadata (needs device_authorization_endpoint)
    /// 2. Requests a device code
    /// 3. Displays the user code and verification URI
    /// 4. Polls the token endpoint until approved or expired
    pub async fn device_flow(&self) -> Result<OAuthTokenResponse, OAuthError> {
        let meta = self.discover().await?;

        let device_endpoint = meta.device_authorization_endpoint.as_ref().ok_or_else(|| {
            OAuthError::Discovery("server does not advertise device_authorization_endpoint".into())
        })?;

        // Request device code
        let params = [("client_id", self.client_id.as_str())];

        let resp = self.http.post(device_endpoint).form(&params).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OAuthError::TokenExchange(format!(
                "device authorization returned {status}: {body}"
            )));
        }

        let device_resp: DeviceAuthResponse = resp.json().await?;

        // Display instructions to user
        println!();
        println!("╔══════════════════════════════════════════════════╗");
        println!("║           Device Authorization Flow              ║");
        println!("╠══════════════════════════════════════════════════╣");
        println!("║                                                  ║");
        println!("║  Visit: {:<40} ║", device_resp.verification_uri);
        println!("║  Code:  {:<40} ║", device_resp.user_code);
        println!("║                                                  ║");
        println!("╚══════════════════════════════════════════════════╝");
        println!();

        if let Some(ref complete_uri) = device_resp.verification_uri_complete {
            println!("Or open: {complete_uri}");
            let _ = open::that(complete_uri);
        }

        println!("Waiting for authorization...");

        // Poll token endpoint
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(device_resp.expires_in);
        let mut interval = std::time::Duration::from_secs(device_resp.interval);

        loop {
            tokio::time::sleep(interval).await;

            if std::time::Instant::now() > deadline {
                return Err(OAuthError::DeviceFlowExpired);
            }

            let params = [
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &device_resp.device_code),
                ("client_id", &self.client_id),
            ];

            let resp = self
                .http
                .post(&meta.token_endpoint)
                .form(&params)
                .send()
                .await?;

            if resp.status().is_success() {
                let tokens: OAuthTokenResponse = resp.json().await?;
                println!("✓ Authorization successful!");
                return Ok(tokens);
            }

            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let error = body["error"].as_str().unwrap_or("unknown");

            match error {
                "authorization_pending" => {
                    // Keep polling
                    print!(".");
                    let _ = std::io::stdout().flush();
                }
                "slow_down" => {
                    interval += std::time::Duration::from_secs(5);
                }
                "expired_token" => {
                    return Err(OAuthError::DeviceFlowExpired);
                }
                "access_denied" => {
                    return Err(OAuthError::TokenExchange("access denied by user".into()));
                }
                _ => {
                    return Err(OAuthError::TokenExchange(format!(
                        "device flow error: {error}"
                    )));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Token refresh
    // -----------------------------------------------------------------------

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
    fn token_store_is_expired_none() {
        let store = TokenStore::default();
        assert!(
            store.is_expired(),
            "None expires_at should be treated as expired"
        );
    }

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
