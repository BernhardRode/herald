//! `herald config` subcommands — show, path, init.

use clap::Subcommand;

use crate::config::Config;

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show current configuration
    Show,
    /// Print the config file path
    Path,
    /// Create a starter config file
    Init,
}

pub async fn handle(cmd: &ConfigCommand) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        ConfigCommand::Show => {
            let config = Config::resolve()?;
            let toml_str = toml::to_string_pretty(&config)?;
            println!("{toml_str}");
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
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let template = r#"# Herald CLI Configuration
# See: https://github.com/bernhardrode/rsjmap/tree/main/crate-herald

default_profile = "default"

[profiles.default]
server_url = "https://mail.example.com"
from_email = "you@example.com"
from_name = "Your Name"

[profiles.default.auth]
method = "app-password"
username = "you@example.com"
password = "your-app-password"

# Alternative: API key
# [profiles.default.auth]
# method = "api-key"
# token = "your-api-key"

# Alternative: OAuth browser flow
# [profiles.default.auth]
# method = "oauth-browser"
# client_id = "herald"

# Alternative: OAuth device flow
# [profiles.default.auth]
# method = "oauth-device"
# client_id = "herald"
"#;
            std::fs::write(&path, template)?;
            println!("✓ Config created at: {}", path.display());
        }
    }
    Ok(())
}
