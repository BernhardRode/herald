//! Configuration error type.

use std::path::PathBuf;

use crate::validate::ValidateError;

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
