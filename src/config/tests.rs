//! Config parsing and profile-resolution tests.

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

fn api_key_profile(server_url: &str, token: &str) -> Profile {
    Profile {
        server_url: server_url.to_string(),
        auth: AuthMethod::ApiKey {
            token: Secret::new(token.to_string()),
        },
        from_email: None,
        from_name: None,
        folders: FolderMappings::default(),
        compose_format: None,
        signature: None,
        allow_insecure: false,
        confirm_actions: true,
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
    let mut profiles = std::collections::HashMap::new();
    profiles.insert(
        "work".to_string(),
        api_key_profile("https://work.example.com", "t"),
    );
    profiles.insert(
        "personal".to_string(),
        api_key_profile("https://personal.example.com", "p"),
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
    let mut profiles = std::collections::HashMap::new();
    profiles.insert(
        "main".to_string(),
        api_key_profile("https://main.example.com", "m"),
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
        profiles: std::collections::HashMap::new(),
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
        profiles: std::collections::HashMap::new(),
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
fn toml_parse_missing_auth_defaults_to_oauth_browser() {
    let toml_str = r#"
        [profiles.default]
        server_url = "https://mail.example.com"
    "#;

    let config: Config = toml::from_str(toml_str).expect("profile without auth should parse");
    let profile = config.profiles.get("default").unwrap();
    match &profile.auth {
        AuthMethod::OAuthBrowser { client_id } => assert_eq!(client_id, "herald"),
        other => panic!("expected OAuthBrowser default, got {other:?}"),
    }
}

#[test]
fn toml_parse_oauth_method_names_and_default_client_id() {
    let toml_str = r#"
        [profiles.browser]
        server_url = "https://a.example.com"

        [profiles.browser.auth]
        method = "oauth-browser"

        [profiles.device]
        server_url = "https://b.example.com"

        [profiles.device.auth]
        method = "oauth-device"
    "#;

    let config: Config = toml::from_str(toml_str).expect("documented method names should parse");
    match &config.profiles.get("browser").unwrap().auth {
        AuthMethod::OAuthBrowser { client_id } => assert_eq!(client_id, "herald"),
        other => panic!("expected OAuthBrowser, got {other:?}"),
    }
    match &config.profiles.get("device").unwrap().auth {
        AuthMethod::OAuthDevice { client_id } => assert_eq!(client_id, "herald"),
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
