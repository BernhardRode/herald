//! herald-jmap-stalwart — Stalwart Mail Server extensions for JMAP clients.
//!
//! Provides OAuth2 PKCE browser flow, device authorization flow, and
//! Stalwart-specific capability detection.

#![forbid(unsafe_code)]

pub mod oauth;

pub use oauth::{DeviceAuthResponse, OAuthMetadata, OAuthTokenResponse, StalwartOAuth, TokenStore};
