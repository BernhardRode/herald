//! Authentication orchestration — connects config profiles to JMAP auth providers.

use std::io::{self, Write};
use std::path::Path;

use jmap_base_client::{BasicAuth, BearerAuth, ClientConfig, DefaultTransport, JmapClient};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use stalwart_rs::{OAuthTokenResponse, StalwartOAuth};

use crate::config::{AuthMethod, Profile};

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("JMAP client error: {0}")]
    Client(#[from] jmap_base_client::ClientError),
    #[error("OAuth error: {0}")]
    OAuth(#[from] stalwart_rs::oauth::OAuthError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config error: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// --- Secure file/directory helpers ---

/// Create a file with restrictive permissions (0600 on Unix).
/// On non-Unix platforms, this falls back to a regular write.
#[cfg(unix)]
pub(crate) fn create_file_secure(path: &Path, content: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(0o600);
    let mut file = opts.open(path)?;
    file.write_all(content)?;
    Ok(())
}

/// Create a file with content. On non-Unix platforms, no special permission
/// enforcement is available — uses a standard write.
#[cfg(not(unix))]
pub(crate) fn create_file_secure(path: &Path, content: &[u8]) -> io::Result<()> {
    std::fs::write(path, content)
}

/// Create a directory (recursively) with restrictive permissions (0700 on Unix).
/// On non-Unix platforms, this falls back to `create_dir_all`.
#[cfg(unix)]
pub(crate) fn create_dir_secure(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

/// Create a directory recursively. On non-Unix platforms, no special permission
/// enforcement is available.
#[cfg(not(unix))]
pub(crate) fn create_dir_secure(path: &Path) -> io::Result<()> {
    std::fs::create_dir_all(path)
}

/// Check file permissions and emit a warning if the file is group- or world-accessible.
/// On Unix, warns if any of the group/other permission bits are set (mode & 0o077 != 0).
/// No-op on non-Unix platforms.
#[cfg(unix)]
pub(crate) fn check_file_permissions(path: &Path) {
    use std::os::unix::fs::MetadataExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.mode();
        if mode & 0o077 != 0 {
            tracing::warn!(
                "File {:?} has insecure permissions {:o} (should be 0600)",
                path,
                mode & 0o777
            );
        }
    }
}

/// No-op on non-Unix platforms — permission checks are not applicable.
#[cfg(not(unix))]
pub(crate) fn check_file_permissions(_path: &Path) {
    // No-op on non-Unix platforms
}

/// Create a JmapClient from a profile's auth configuration.
pub async fn create_client(profile: &Profile, profile_name: &str) -> Result<JmapClient, AuthError> {
    match &profile.auth {
        AuthMethod::AppPassword { username, password } => {
            let auth = BasicAuth::new(username, password.expose())?;
            let client = JmapClient::new(
                DefaultTransport,
                auth,
                &profile.server_url,
                ClientConfig::default(),
            )?;
            Ok(client)
        }
        AuthMethod::ApiKey { token } => {
            let auth = BearerAuth::new(token.expose())?;
            let client = JmapClient::new(
                DefaultTransport,
                auth,
                &profile.server_url,
                ClientConfig::default(),
            )?;
            Ok(client)
        }
        AuthMethod::OAuthBrowser { client_id } => {
            let tokens = oauth_browser_login(&profile.server_url, client_id, profile_name).await?;
            let auth = BearerAuth::new(&tokens.access_token)?;
            let client = JmapClient::new(
                DefaultTransport,
                auth,
                &profile.server_url,
                ClientConfig::default(),
            )?;
            Ok(client)
        }
        AuthMethod::OAuthDevice { client_id } => {
            let tokens = oauth_device_login(&profile.server_url, client_id, profile_name).await?;
            let auth = BearerAuth::new(&tokens.access_token)?;
            let client = JmapClient::new(
                DefaultTransport,
                auth,
                &profile.server_url,
                ClientConfig::default(),
            )?;
            Ok(client)
        }
    }
}

/// Perform OAuth browser flow, caching tokens if possible.
async fn oauth_browser_login(
    server_url: &str,
    client_id: &str,
    profile_name: &str,
) -> Result<OAuthTokenResponse, AuthError> {
    // Check for cached tokens first
    if let Some(store) = load_token_store(profile_name, server_url) {
        if !store.is_expired() {
            if let Some(ref token) = store.access_token {
                return Ok(OAuthTokenResponse {
                    access_token: token.clone(),
                    token_type: Some("Bearer".into()),
                    expires_in: None,
                    refresh_token: store.refresh_token.clone(),
                    scope: None,
                });
            }
        }
        // Try refresh
        if let Some(ref refresh) = store.refresh_token {
            let oauth = StalwartOAuth::new(server_url, client_id)?;
            match oauth.refresh_token(refresh).await {
                Ok(tokens) => {
                    save_token_store(profile_name, server_url, &tokens);
                    return Ok(tokens);
                }
                Err(e) => {
                    tracing::warn!("Token refresh failed, re-authenticating: {e}");
                }
            }
        }
    }

    let oauth = StalwartOAuth::new(server_url, client_id)?;
    let tokens = oauth.browser_flow().await?;
    save_token_store(profile_name, server_url, &tokens);
    Ok(tokens)
}

/// Perform OAuth device flow, caching tokens if possible.
async fn oauth_device_login(
    server_url: &str,
    client_id: &str,
    profile_name: &str,
) -> Result<OAuthTokenResponse, AuthError> {
    // Check for cached tokens first
    if let Some(store) = load_token_store(profile_name, server_url) {
        if !store.is_expired() {
            if let Some(ref token) = store.access_token {
                return Ok(OAuthTokenResponse {
                    access_token: token.clone(),
                    token_type: Some("Bearer".into()),
                    expires_in: None,
                    refresh_token: store.refresh_token.clone(),
                    scope: None,
                });
            }
        }
        // Try refresh
        if let Some(ref refresh) = store.refresh_token {
            let oauth = StalwartOAuth::new(server_url, client_id)?;
            match oauth.refresh_token(refresh).await {
                Ok(tokens) => {
                    save_token_store(profile_name, server_url, &tokens);
                    return Ok(tokens);
                }
                Err(e) => {
                    tracing::warn!("Token refresh failed, re-authenticating: {e}");
                }
            }
        }
    }

    let oauth = StalwartOAuth::new(server_url, client_id)?;
    let tokens = oauth.device_flow().await?;
    save_token_store(profile_name, server_url, &tokens);
    Ok(tokens)
}

// ---------------------------------------------------------------------------
// Token store V2 — profile-scoped with server URL validation
// ---------------------------------------------------------------------------

/// Version 2 of the token store, keyed by profile and server URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TokenStoreV2 {
    /// Schema version (always 2)
    pub version: u8,
    /// The server URL this token was issued for
    pub server_url: String,
    /// The profile name this token belongs to
    pub profile_name: String,
    /// OAuth access token
    pub access_token: Option<String>,
    /// OAuth refresh token
    pub refresh_token: Option<String>,
    /// Unix timestamp when the access token expires
    pub expires_at: Option<u64>,
}

impl TokenStoreV2 {
    /// 60-second skew buffer: treat tokens expiring within 60s as already expired.
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

fn token_store_path(profile_name: &str, server_url: &str) -> Option<std::path::PathBuf> {
    let mut hasher = Sha256::new();
    hasher.update(server_url.as_bytes());
    let result = hasher.finalize();
    let hash: String = result.iter().map(|b| format!("{b:02x}")).collect();
    let short_hash = &hash[..8];

    crate::config::Config::token_dir()
        .ok()
        .map(|d| d.join(format!("{profile_name}-{short_hash}.json")))
}

fn load_token_store(profile_name: &str, server_url: &str) -> Option<TokenStoreV2> {
    let path = token_store_path(profile_name, server_url)?;

    // Check file permissions on load
    check_file_permissions(&path);

    let content = std::fs::read_to_string(&path).ok()?;
    let store: TokenStoreV2 = serde_json::from_str(&content).ok()?;

    // Verify server_url matches — discard on mismatch
    if store.server_url != server_url {
        tracing::warn!(
            "Token cache {:?} has server_url mismatch (stored={}, expected={}), discarding",
            path,
            store.server_url,
            server_url
        );
        return None;
    }

    Some(store)
}

fn save_token_store(profile_name: &str, server_url: &str, tokens: &OAuthTokenResponse) {
    let Some(path) = token_store_path(profile_name, server_url) else {
        tracing::warn!("Cannot determine token store path");
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = create_dir_secure(parent) {
            tracing::warn!("Failed to create token directory {:?}: {}", parent, e);
            return;
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let store = TokenStoreV2 {
        version: 2,
        server_url: server_url.to_string(),
        profile_name: profile_name.to_string(),
        access_token: Some(tokens.access_token.clone()),
        refresh_token: tokens.refresh_token.clone(),
        expires_at: tokens.expires_in.map(|e| now + e),
    };
    match serde_json::to_string_pretty(&store) {
        Ok(json) => {
            if let Err(e) = create_file_secure(&path, json.as_bytes()) {
                tracing::warn!("Failed to write token cache {:?}: {}", path, e);
            }
        }
        Err(e) => {
            tracing::warn!("Failed to serialize token store: {}", e);
        }
    }
}
