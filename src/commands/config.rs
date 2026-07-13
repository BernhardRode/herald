//! `herald config` subcommands — show, path, init.

use clap::Subcommand;

use crate::auth::{create_dir_secure, create_file_secure};
use crate::config::{AuthMethod, Config};

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show current configuration
    Show {
        /// Show actual secret values (passwords, tokens) instead of "***"
        #[arg(long)]
        reveal: bool,
    },
    /// Print the config file path
    Path,
    /// Create a starter config file
    Init,
    /// Print the .env.example template (embedded at build time)
    Env,
}

pub async fn handle(cmd: &ConfigCommand) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        ConfigCommand::Show { reveal } => {
            let config = Config::resolve()?;
            if *reveal {
                let revealed = reveal_config(&config);
                println!("{revealed}");
            } else {
                // Default: Secret<String> serializes to "***"
                let toml_str = toml::to_string_pretty(&config)?;
                println!("{toml_str}");
            }
        }
        ConfigCommand::Path => {
            let path = Config::config_path()?;
            println!("{}", path.display());
        }
        ConfigCommand::Init => {
            let path = Config::config_path()?;
            if path.exists() {
                println!("Config already exists at: {}", path.display());
                return Ok(());
            }
            // Ensure parent directory exists with secure permissions (0700 on Unix)
            if let Some(parent) = path.parent() {
                create_dir_secure(parent)?;
            }
            // Write config with secure permissions (0600 on Unix)
            create_file_secure(&path, crate::config::CONFIG_EXAMPLE.as_bytes())?;
            println!("✓ Config created at: {}", path.display());
        }
        ConfigCommand::Env => {
            print!("{}", crate::config::ENV_EXAMPLE);
        }
    }
    Ok(())
}

/// Serialize the config with secret values exposed (for `--reveal`).
///
/// Builds a `toml::Value` from the config (which redacts secrets by default),
/// then walks through profiles and replaces "***" with actual exposed values.
fn reveal_config(config: &Config) -> String {
    let mut value = match toml::Value::try_from(config) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    if let Some(profiles) = value.get_mut("profiles").and_then(|p| p.as_table_mut()) {
        for (name, profile_val) in profiles.iter_mut() {
            if let Some(profile) = config.profiles.get(name) {
                if let Some(auth) = profile_val.get_mut("auth").and_then(|a| a.as_table_mut()) {
                    match &profile.auth {
                        AuthMethod::AppPassword { password, .. } => {
                            auth.insert(
                                "password".to_string(),
                                toml::Value::String(password.expose().clone()),
                            );
                        }
                        AuthMethod::ApiKey { token } => {
                            auth.insert(
                                "token".to_string(),
                                toml::Value::String(token.expose().clone()),
                            );
                        }
                        AuthMethod::OAuthBrowser { .. } | AuthMethod::OAuthDevice { .. } => {
                            // No secrets to reveal for OAuth methods
                        }
                    }
                }
            }
        }
    }

    toml::to_string_pretty(&value).unwrap_or_default()
}
