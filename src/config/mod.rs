//! XDG-compliant TOML configuration with multi-profile support.
//!
//! Config file location: `$XDG_CONFIG_HOME/herald/config.toml`
//! (defaults to `~/.config/herald/config.toml`)
//!
//! Token cache location: `$XDG_DATA_HOME/herald/tokens/<profile>-<server-hash>.json`

mod error;
mod profile;
#[cfg(test)]
mod tests;

pub use error::ConfigError;
pub use profile::{AuthMethod, FolderMappings, Profile};

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::auth::check_file_permissions;
use crate::secret::Secret;
use crate::validate::validate_server_url;

/// The `.env.example` template, embedded at compile time.
pub const ENV_EXAMPLE: &str = include_str!("../../.env.example");

/// The `config.example.toml` template, embedded at compile time.
pub const CONFIG_EXAMPLE: &str = include_str!("../../config.example.toml");

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
            confirm_actions: true,
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
        self.get_profile_with_name(name).map(|(_, p)| p)
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
