//! XDG-compliant TOML configuration with multi-profile support.
//!
//! Config file location: `$XDG_CONFIG_HOME/herald/config.toml`
//! (defaults to `~/.config/herald/config.toml`)
//!
//! Token cache location: `$XDG_DATA_HOME/herald/tokens/<profile>.json`

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::auth::check_file_permissions;
use crate::secret::Secret;
use crate::validate::{validate_server_url, ValidateError};

/// Authentication method for a profile.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum AuthMethod {
    /// HTTP Basic auth with username:password (app password)
    AppPassword {
        username: String,
        password: Secret<String>,
    },
    /// Bearer token (API key)
    ApiKey { token: Secret<String> },
    /// OAuth2 browser flow (Authorization Code + PKCE)
    OAuthBrowser { client_id: String },
    /// OAuth2 Device Authorization Grant
    OAuthDevice { client_id: String },
}

impl std::fmt::Debug for AuthMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AppPassword { username, .. } => f
                .debug_struct("AppPassword")
                .field("username", username)
                .field("password", &"***")
                .finish(),
            Self::ApiKey { .. } => f.debug_struct("ApiKey").field("token", &"***").finish(),
            Self::OAuthBrowser { client_id } => f
                .debug_struct("OAuthBrowser")
                .field("client_id", client_id)
                .finish(),
            Self::OAuthDevice { client_id } => f
                .debug_struct("OAuthDevice")
                .field("client_id", client_id)
                .finish(),
        }
    }
}

/// A named connection profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// JMAP server URL (e.g. "https://mail.example.com")
    pub server_url: String,
    /// Authentication configuration
    pub auth: AuthMethod,
    /// Default "From" email address for this profile
    #[serde(default)]
    pub from_email: Option<String>,
    /// Default "From" display name
    #[serde(default)]
    pub from_name: Option<String>,
    /// Folder mappings for mail actions (archive, spam, trash).
    /// Values are mailbox names or paths (e.g. "Archive/2026").
    #[serde(default)]
    pub folders: FolderMappings,
    /// Format for composing emails: "plain" (default) or "markdown" (converts to HTML).
    #[serde(default)]
    pub compose_format: Option<String>,
    /// Email signature appended to new messages and replies.
    #[serde(default)]
    pub signature: Option<String>,
    /// Allow non-HTTPS server URLs (for local development).
    #[serde(default)]
    pub allow_insecure: bool,
}

/// Configurable folder mappings for mail actions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FolderMappings {
    /// Mailbox name/path for archiving (default: "Archive")
    #[serde(default)]
    pub archive: Option<String>,
    /// Mailbox name/path for spam/junk (default: "Junk")
    #[serde(default)]
    pub spam: Option<String>,
    /// Mailbox name/path for trash/deleted (default: "Trash")
    #[serde(default)]
    pub trash: Option<String>,
}

/// Top-level config file structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Default profile name to use when --profile is not specified
    #[serde(default)]
    pub default_profile: Option<String>,
    /// Named profiles
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,
}

impl Config {
    /// Load config from the standard XDG path.
    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        check_file_permissions(&path);
        let content =
            std::fs::read_to_string(&path).map_err(|e| ConfigError::Io(path.clone(), e))?;
        let config: Config = toml::from_str(&content).map_err(|e| ConfigError::Parse(path, e))?;

        // Validate server URLs for each profile
        for profile in config.profiles.values() {
            validate_server_url(&profile.server_url, profile.allow_insecure)
                .map_err(ConfigError::Validate)?;
        }

        Ok(config)
    }

    /// Load config from environment variables (for .env-based usage).
    ///
    /// Env vars:
    /// - HERALD_SERVER_URL
    /// - HERALD_AUTH_METHOD (app-password | api-key | oauth-browser | oauth-device)
    /// - HERALD_USERNAME
    /// - HERALD_PASSWORD
    /// - HERALD_API_KEY
    /// - HERALD_CLIENT_ID
    /// - HERALD_FROM_EMAIL
    /// - HERALD_FROM_NAME
    pub fn from_env() -> Result<Self, ConfigError> {
        let server_url = std::env::var("HERALD_SERVER_URL")
            .map_err(|_| ConfigError::MissingEnv("HERALD_SERVER_URL"))?;

        // Validate the server URL (allow_insecure defaults to false for env-based config)
        validate_server_url(&server_url, false).map_err(ConfigError::Validate)?;

        let auth_method =
            std::env::var("HERALD_AUTH_METHOD").unwrap_or_else(|_| "app-password".into());

        let auth = match auth_method.as_str() {
            "app-password" => {
                let username = std::env::var("HERALD_USERNAME")
                    .map_err(|_| ConfigError::MissingEnv("HERALD_USERNAME"))?;
                let password = std::env::var("HERALD_PASSWORD")
                    .map_err(|_| ConfigError::MissingEnv("HERALD_PASSWORD"))?;
                AuthMethod::AppPassword {
                    username,
                    password: Secret::new(password),
                }
            }
            "api-key" => {
                let token = std::env::var("HERALD_API_KEY")
                    .map_err(|_| ConfigError::MissingEnv("HERALD_API_KEY"))?;
                AuthMethod::ApiKey {
                    token: Secret::new(token),
                }
            }
            "oauth-browser" => {
                let client_id = std::env::var("HERALD_CLIENT_ID")
                    .map_err(|_| ConfigError::MissingEnv("HERALD_CLIENT_ID"))?;
                AuthMethod::OAuthBrowser { client_id }
            }
            "oauth-device" => {
                let client_id = std::env::var("HERALD_CLIENT_ID")
                    .map_err(|_| ConfigError::MissingEnv("HERALD_CLIENT_ID"))?;
                AuthMethod::OAuthDevice { client_id }
            }
            other => {
                return Err(ConfigError::InvalidAuthMethod(other.to_string()));
            }
        };

        let from_email = std::env::var("HERALD_FROM_EMAIL").ok();
        let from_name = std::env::var("HERALD_FROM_NAME").ok();

        let profile = Profile {
            server_url,
            auth,
            from_email,
            from_name,
            folders: FolderMappings::default(),
            compose_format: None,
            signature: None,
            allow_insecure: false,
        };

        let mut profiles = HashMap::new();
        profiles.insert("default".to_string(), profile);

        Ok(Config {
            default_profile: Some("default".to_string()),
            profiles,
        })
    }

    /// Resolve the effective config: try TOML file first, fall back to env vars.
    pub fn resolve() -> Result<Self, ConfigError> {
        let file_config = Self::load()?;
        if !file_config.profiles.is_empty() {
            return Ok(file_config);
        }
        Self::from_env()
    }

    /// Get a profile by name (or the default profile).
    pub fn get_profile(&self, name: Option<&str>) -> Result<&Profile, ConfigError> {
        let profile_name = name
            .map(|s| s.to_string())
            .or_else(|| self.default_profile.clone())
            .unwrap_or_else(|| "default".to_string());

        self.profiles
            .get(&profile_name)
            .ok_or(ConfigError::ProfileNotFound(profile_name))
    }

    /// Get a profile by name (or the default profile) along with the resolved profile name.
    pub fn get_profile_with_name(
        &self,
        name: Option<&str>,
    ) -> Result<(&str, &Profile), ConfigError> {
        let profile_name = name
            .map(|s| s.to_string())
            .or_else(|| self.default_profile.clone())
            .unwrap_or_else(|| "default".to_string());

        self.profiles
            .get_key_value(&profile_name)
            .map(|(k, v)| (k.as_str(), v))
            .ok_or(ConfigError::ProfileNotFound(profile_name))
    }

    /// Standard config file path.
    pub fn config_path() -> Result<PathBuf, ConfigError> {
        let config_dir = dirs::config_dir()
            .ok_or(ConfigError::NoConfigDir)?
            .join("herald");
        Ok(config_dir.join("config.toml"))
    }

    /// Token storage directory.
    pub fn token_dir() -> Result<PathBuf, ConfigError> {
        let data_dir = dirs::data_dir()
            .ok_or(ConfigError::NoConfigDir)?
            .join("herald")
            .join("tokens");
        Ok(data_dir)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file IO error at {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("config parse error in {0}: {1}")]
    Parse(PathBuf, toml::de::Error),
    #[error("profile not found: {0}")]
    ProfileNotFound(String),
    #[error("missing environment variable: {0}")]
    MissingEnv(&'static str),
    #[error("invalid auth method: {0}")]
    InvalidAuthMethod(String),
    #[error("could not determine config directory")]
    NoConfigDir,
    #[error("URL validation error: {0}")]
    Validate(#[from] ValidateError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Secret;
    use std::sync::Mutex;

    /// Global mutex to serialize tests that mutate environment variables.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// Helper: clear all HERALD_* env vars to avoid cross-test contamination.
    fn clear_herald_env() {
        for var in &[
            "HERALD_SERVER_URL",
            "HERALD_AUTH_METHOD",
            "HERALD_USERNAME",
            "HERALD_PASSWORD",
            "HERALD_API_KEY",
            "HERALD_CLIENT_ID",
            "HERALD_FROM_EMAIL",
            "HERALD_FROM_NAME",
        ] {
            unsafe { std::env::remove_var(var) };
        }
    }

    /// Helper: set env vars from a slice of (key, value) pairs.
    fn set_env(vars: &[(&str, &str)]) {
        for (k, v) in vars {
            unsafe { std::env::set_var(k, v) };
        }
    }

    // ─── from_env: app-password ──────────────────────────────────────────

    #[test]
    fn from_env_app_password() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_herald_env();
        set_env(&[
            ("HERALD_SERVER_URL", "https://mail.example.com"),
            ("HERALD_AUTH_METHOD", "app-password"),
            ("HERALD_USERNAME", "alice"),
            ("HERALD_PASSWORD", "secret123"),
            ("HERALD_FROM_EMAIL", "alice@example.com"),
            ("HERALD_FROM_NAME", "Alice"),
        ]);

        let config = Config::from_env().expect("from_env should succeed");
        let profile = config
            .get_profile(None)
            .expect("default profile should exist");

        assert_eq!(profile.server_url, "https://mail.example.com");
        assert_eq!(profile.from_email.as_deref(), Some("alice@example.com"));
        assert_eq!(profile.from_name.as_deref(), Some("Alice"));
        match &profile.auth {
            AuthMethod::AppPassword { username, password } => {
                assert_eq!(username, "alice");
                assert_eq!(password.expose(), "secret123");
            }
            other => panic!("expected AppPassword, got {other:?}"),
        }

        clear_herald_env();
    }

    // ─── from_env: api-key ───────────────────────────────────────────────

    #[test]
    fn from_env_api_key() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_herald_env();
        set_env(&[
            ("HERALD_SERVER_URL", "https://mail.example.com"),
            ("HERALD_AUTH_METHOD", "api-key"),
            ("HERALD_API_KEY", "tok_abc123"),
        ]);

        let config = Config::from_env().expect("from_env should succeed");
        let profile = config.get_profile(None).unwrap();

        match &profile.auth {
            AuthMethod::ApiKey { token } => {
                assert_eq!(token.expose(), "tok_abc123");
            }
            other => panic!("expected ApiKey, got {other:?}"),
        }

        clear_herald_env();
    }

    // ─── from_env: oauth-browser ─────────────────────────────────────────

    #[test]
    fn from_env_oauth_browser() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_herald_env();
        set_env(&[
            ("HERALD_SERVER_URL", "https://mail.example.com"),
            ("HERALD_AUTH_METHOD", "oauth-browser"),
            ("HERALD_CLIENT_ID", "my-client"),
        ]);

        let config = Config::from_env().expect("from_env should succeed");
        let profile = config.get_profile(None).unwrap();

        match &profile.auth {
            AuthMethod::OAuthBrowser { client_id } => {
                assert_eq!(client_id, "my-client");
            }
            other => panic!("expected OAuthBrowser, got {other:?}"),
        }

        clear_herald_env();
    }

    // ─── from_env: oauth-device ──────────────────────────────────────────

    #[test]
    fn from_env_oauth_device() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_herald_env();
        set_env(&[
            ("HERALD_SERVER_URL", "https://mail.example.com"),
            ("HERALD_AUTH_METHOD", "oauth-device"),
            ("HERALD_CLIENT_ID", "device-client"),
        ]);

        let config = Config::from_env().expect("from_env should succeed");
        let profile = config.get_profile(None).unwrap();

        match &profile.auth {
            AuthMethod::OAuthDevice { client_id } => {
                assert_eq!(client_id, "device-client");
            }
            other => panic!("expected OAuthDevice, got {other:?}"),
        }

        clear_herald_env();
    }

    // ─── from_env: missing server_url gives error ────────────────────────

    #[test]
    fn from_env_missing_server_url() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_herald_env();

        let err = Config::from_env().unwrap_err();
        assert!(
            matches!(err, ConfigError::MissingEnv("HERALD_SERVER_URL")),
            "expected MissingEnv(HERALD_SERVER_URL), got {err:?}"
        );
    }

    // ─── from_env: invalid auth method ───────────────────────────────────

    #[test]
    fn from_env_invalid_auth_method() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_herald_env();
        set_env(&[
            ("HERALD_SERVER_URL", "https://mail.example.com"),
            ("HERALD_AUTH_METHOD", "bogus"),
        ]);

        let err = Config::from_env().unwrap_err();
        match err {
            ConfigError::InvalidAuthMethod(method) => assert_eq!(method, "bogus"),
            other => panic!("expected InvalidAuthMethod, got {other:?}"),
        }

        clear_herald_env();
    }

    // ─── Profile resolution ──────────────────────────────────────────────

    #[test]
    fn get_profile_by_name() {
        let mut profiles = HashMap::new();
        profiles.insert(
            "work".to_string(),
            Profile {
                server_url: "https://work.example.com".to_string(),
                auth: AuthMethod::ApiKey {
                    token: Secret::new("t".to_string()),
                },
                from_email: None,
                from_name: None,
                folders: FolderMappings::default(),
                compose_format: None,
                signature: None,
                allow_insecure: false,
            },
        );
        profiles.insert(
            "personal".to_string(),
            Profile {
                server_url: "https://personal.example.com".to_string(),
                auth: AuthMethod::ApiKey {
                    token: Secret::new("p".to_string()),
                },
                from_email: None,
                from_name: None,
                folders: FolderMappings::default(),
                compose_format: None,
                signature: None,
                allow_insecure: false,
            },
        );

        let config = Config {
            default_profile: Some("work".to_string()),
            profiles,
        };

        let profile = config.get_profile(Some("personal")).unwrap();
        assert_eq!(profile.server_url, "https://personal.example.com");
    }

    #[test]
    fn get_profile_uses_default_when_none() {
        let mut profiles = HashMap::new();
        profiles.insert(
            "main".to_string(),
            Profile {
                server_url: "https://main.example.com".to_string(),
                auth: AuthMethod::ApiKey {
                    token: Secret::new("m".to_string()),
                },
                from_email: None,
                from_name: None,
                folders: FolderMappings::default(),
                compose_format: None,
                signature: None,
                allow_insecure: false,
            },
        );

        let config = Config {
            default_profile: Some("main".to_string()),
            profiles,
        };

        let profile = config.get_profile(None).unwrap();
        assert_eq!(profile.server_url, "https://main.example.com");
    }

    #[test]
    fn get_profile_missing_returns_error() {
        let config = Config {
            default_profile: None,
            profiles: HashMap::new(),
        };

        let err = config.get_profile(Some("nonexistent")).unwrap_err();
        match err {
            ConfigError::ProfileNotFound(name) => assert_eq!(name, "nonexistent"),
            other => panic!("expected ProfileNotFound, got {other:?}"),
        }
    }

    #[test]
    fn get_profile_no_default_falls_back_to_literal_default() {
        // When no name is given and no default_profile is set,
        // it looks for a profile named "default".
        let config = Config {
            default_profile: None,
            profiles: HashMap::new(),
        };

        let err = config.get_profile(None).unwrap_err();
        match err {
            ConfigError::ProfileNotFound(name) => assert_eq!(name, "default"),
            other => panic!("expected ProfileNotFound(\"default\"), got {other:?}"),
        }
    }

    // ─── TOML parsing ────────────────────────────────────────────────────

    #[test]
    fn toml_parse_full_config() {
        let toml_str = r#"
            default_profile = "home"

            [profiles.home]
            server_url = "https://home.example.com"
            from_email = "me@home.example.com"
            from_name = "Me"

            [profiles.home.auth]
            method = "app-password"
            username = "me"
            password = "hunter2"

            [profiles.work]
            server_url = "https://work.corp.com"

            [profiles.work.auth]
            method = "o-auth-browser"
            client_id = "corp-app"
        "#;

        let config: Config = toml::from_str(toml_str).expect("TOML parse should succeed");

        assert_eq!(config.default_profile.as_deref(), Some("home"));
        assert_eq!(config.profiles.len(), 2);

        let home = config.profiles.get("home").unwrap();
        assert_eq!(home.server_url, "https://home.example.com");
        assert_eq!(home.from_email.as_deref(), Some("me@home.example.com"));
        assert_eq!(home.from_name.as_deref(), Some("Me"));
        match &home.auth {
            AuthMethod::AppPassword { username, password } => {
                assert_eq!(username, "me");
                assert_eq!(password.expose(), "hunter2");
            }
            other => panic!("expected AppPassword, got {other:?}"),
        }

        let work = config.profiles.get("work").unwrap();
        assert_eq!(work.server_url, "https://work.corp.com");
        assert!(work.from_email.is_none());
        match &work.auth {
            AuthMethod::OAuthBrowser { client_id } => {
                assert_eq!(client_id, "corp-app");
            }
            other => panic!("expected OAuthBrowser, got {other:?}"),
        }
    }

    #[test]
    fn toml_parse_api_key_and_oauth_device() {
        let toml_str = r#"
            [profiles.api]
            server_url = "https://api.example.com"

            [profiles.api.auth]
            method = "api-key"
            token = "secret-token-xyz"

            [profiles.device]
            server_url = "https://device.example.com"

            [profiles.device.auth]
            method = "o-auth-device"
            client_id = "device-id-123"
        "#;

        let config: Config = toml::from_str(toml_str).expect("TOML parse should succeed");

        let api = config.profiles.get("api").unwrap();
        match &api.auth {
            AuthMethod::ApiKey { token } => assert_eq!(token.expose(), "secret-token-xyz"),
            other => panic!("expected ApiKey, got {other:?}"),
        }

        let device = config.profiles.get("device").unwrap();
        match &device.auth {
            AuthMethod::OAuthDevice { client_id } => assert_eq!(client_id, "device-id-123"),
            other => panic!("expected OAuthDevice, got {other:?}"),
        }
    }

    #[test]
    fn toml_parse_empty_config() {
        let config: Config = toml::from_str("").expect("empty TOML should parse to default");
        assert!(config.default_profile.is_none());
        assert!(config.profiles.is_empty());
    }

    // ─── config_path ─────────────────────────────────────────────────────

    #[test]
    fn config_path_returns_valid_path() {
        let path = Config::config_path().expect("config_path should succeed");
        // Must end with the expected filename
        assert_eq!(path.file_name().unwrap(), "config.toml");
        // Must be inside a "herald" directory
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "herald");
        // Must be an absolute path
        assert!(path.is_absolute());
    }
}
