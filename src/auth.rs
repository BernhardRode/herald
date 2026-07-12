//! Authentication orchestration — connects config profiles to JMAP auth providers.

use jmap_base_client::{BasicAuth, BearerAuth, ClientConfig, DefaultTransport, JmapClient};
use stalwart_rs::{OAuthTokenResponse, StalwartOAuth, TokenStore};

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

/// Create a JmapClient from a profile's auth configuration.
pub async fn create_client(profile: &Profile) -> Result<JmapClient, AuthError> {
    match &profile.auth {
        AuthMethod::AppPassword { username, password } => {
            let auth = BasicAuth::new(username, password)?;
            let client = JmapClient::new(
                DefaultTransport,
                auth,
                &profile.server_url,
                ClientConfig::default(),
            )?;
            Ok(client)
        }
        AuthMethod::ApiKey { token } => {
            let auth = BearerAuth::new(token)?;
            let client = JmapClient::new(
                DefaultTransport,
                auth,
                &profile.server_url,
                ClientConfig::default(),
            )?;
            Ok(client)
        }
        AuthMethod::OAuthBrowser { client_id } => {
            let tokens = oauth_browser_login(&profile.server_url, client_id).await?;
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
            let tokens = oauth_device_login(&profile.server_url, client_id).await?;
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
) -> Result<OAuthTokenResponse, AuthError> {
    // Check for cached tokens first
    if let Some(store) = load_token_store("oauth_browser") {
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
                    save_token_store("oauth_browser", &tokens);
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
    save_token_store("oauth_browser", &tokens);
    Ok(tokens)
}

/// Perform OAuth device flow, caching tokens if possible.
async fn oauth_device_login(
    server_url: &str,
    client_id: &str,
) -> Result<OAuthTokenResponse, AuthError> {
    // Check for cached tokens first
    if let Some(store) = load_token_store("oauth_device") {
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
                    save_token_store("oauth_device", &tokens);
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
    save_token_store("oauth_device", &tokens);
    Ok(tokens)
}

fn token_store_path(name: &str) -> Option<std::path::PathBuf> {
    crate::config::Config::token_dir()
        .ok()
        .map(|d| d.join(format!("{name}.json")))
}

fn load_token_store(name: &str) -> Option<TokenStore> {
    let path = token_store_path(name)?;
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_token_store(name: &str, tokens: &OAuthTokenResponse) {
    let Some(path) = token_store_path(name) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let store = TokenStore {
        access_token: Some(tokens.access_token.clone()),
        refresh_token: tokens.refresh_token.clone(),
        expires_at: tokens.expires_in.map(|e| now + e),
    };
    if let Ok(json) = serde_json::to_string_pretty(&store) {
        let _ = std::fs::write(path, json);
    }
}
