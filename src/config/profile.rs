//! Profile types: auth methods, folder mappings, per-profile settings.

use serde::{Deserialize, Serialize};

use crate::secret::Secret;

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
    /// Whether destructive actions (delete, archive, spam) require y/n confirmation.
    /// Defaults to `true`. Set to `false` to skip confirmations.
    #[serde(default = "default_true")]
    pub confirm_actions: bool,
}

fn default_true() -> bool {
    true
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
