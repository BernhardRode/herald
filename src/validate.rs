//! URL scheme validation and header value injection prevention.

use url::{Host, Url};

/// Validate that a server URL uses HTTPS (or is loopback for development).
///
/// Accepts the URL if:
/// - `allow_insecure` is true (any scheme permitted), or
/// - the scheme is `https`, or
/// - the scheme is `http` AND the host is a loopback address (127.0.0.1, ::1, localhost).
///
/// All other combinations are rejected with [`ValidateError::InsecureUrl`].
pub fn validate_server_url(url: &str, allow_insecure: bool) -> Result<(), ValidateError> {
    let parsed = Url::parse(url)?;
    if allow_insecure {
        return Ok(());
    }
    if parsed.scheme() == "https" {
        return Ok(());
    }
    // Allow http for loopback only
    if parsed.scheme() == "http" && is_loopback(&parsed) {
        return Ok(());
    }
    Err(ValidateError::InsecureUrl(url.to_string()))
}

/// Check whether a parsed URL points to a loopback address.
fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain == "localhost",
        Some(Host::Ipv4(addr)) => addr.is_loopback(),
        Some(Host::Ipv6(addr)) => addr.is_loopback(),
        None => false,
    }
}

/// Validate that a header value contains no CR or LF characters.
///
/// Rejects values containing `\r` (0x0D) or `\n` (0x0A) to prevent header injection attacks.
pub fn validate_header_value(name: &str, value: &str) -> Result<(), ValidateError> {
    if value.contains('\r') || value.contains('\n') {
        Err(ValidateError::HeaderInjection(name.to_string()))
    } else {
        Ok(())
    }
}

/// Errors produced by input validation functions.
#[derive(Debug, thiserror::Error)]
pub enum ValidateError {
    /// The server URL does not use HTTPS and is not a loopback address.
    #[error("HTTPS required for server URL '{0}'. Use allow_insecure = true to override.")]
    InsecureUrl(String),
    /// A header value contains CR/LF characters, indicating possible injection.
    #[error("header '{0}' contains CR/LF — possible injection")]
    HeaderInjection(String),
    /// The URL could not be parsed.
    #[error("invalid URL: {0}")]
    UrlParse(#[from] url::ParseError),
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- validate_server_url tests ---

    #[test]
    fn accepts_https_url() {
        assert!(validate_server_url("https://mail.example.com", false).is_ok());
    }

    #[test]
    fn accepts_https_with_path_and_port() {
        assert!(validate_server_url("https://mail.example.com:8443/jmap", false).is_ok());
    }

    #[test]
    fn rejects_http_remote_host() {
        let err = validate_server_url("http://mail.example.com", false).unwrap_err();
        assert!(matches!(err, ValidateError::InsecureUrl(_)));
    }

    #[test]
    fn accepts_http_localhost() {
        assert!(validate_server_url("http://localhost:8080", false).is_ok());
    }

    #[test]
    fn accepts_http_ipv4_loopback() {
        assert!(validate_server_url("http://127.0.0.1:1080", false).is_ok());
    }

    #[test]
    fn accepts_http_ipv6_loopback() {
        assert!(validate_server_url("http://[::1]:8080", false).is_ok());
    }

    #[test]
    fn allow_insecure_permits_any_scheme() {
        assert!(validate_server_url("http://remote.server.io", true).is_ok());
        assert!(validate_server_url("ftp://files.example.com", true).is_ok());
    }

    #[test]
    fn rejects_invalid_url() {
        let err = validate_server_url("not a url at all", false).unwrap_err();
        assert!(matches!(err, ValidateError::UrlParse(_)));
    }

    // --- validate_header_value tests ---

    #[test]
    fn accepts_normal_header_value() {
        assert!(validate_header_value("subject", "Hello World").is_ok());
    }

    #[test]
    fn accepts_unicode_header_value() {
        assert!(validate_header_value("subject", "Greetings from Munchen").is_ok());
    }

    #[test]
    fn rejects_cr_in_header_value() {
        let err = validate_header_value("to", "user@example.com\rBcc: spy@evil.com").unwrap_err();
        assert!(matches!(err, ValidateError::HeaderInjection(_)));
    }

    #[test]
    fn rejects_lf_in_header_value() {
        let err = validate_header_value("subject", "Hello\nBcc: spy@evil.com").unwrap_err();
        assert!(matches!(err, ValidateError::HeaderInjection(_)));
    }

    #[test]
    fn rejects_crlf_in_header_value() {
        let err = validate_header_value("from", "attacker\r\n injected: true").unwrap_err();
        assert!(matches!(err, ValidateError::HeaderInjection(_)));
    }

    #[test]
    fn accepts_empty_header_value() {
        assert!(validate_header_value("x-custom", "").is_ok());
    }
}
