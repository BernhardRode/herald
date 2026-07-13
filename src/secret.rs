//! Secret<T> newtype that redacts sensitive values in Debug, Display, and Serialize.
//!
//! Prevents accidental exposure of credentials in logs, serialized output, and terminal display.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// A secret value that is redacted in Debug, Display, and default Serialize.
#[derive(Clone)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// Wrap a value as a secret.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Expose the inner secret value. Use sparingly — only when the actual value is needed
    /// (e.g., sending credentials to a server, or when `--reveal` is specified).
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl<T: fmt::Display> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl<T: Serialize> Serialize for Secret<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("***")
    }
}

impl<'de> Deserialize<'de> for Secret<String> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Secret(s))
    }
}

impl<T: PartialEq> PartialEq for Secret<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_redacted() {
        let secret = Secret::new("my-password".to_string());
        let debug_output = format!("{:?}", secret);
        assert_eq!(debug_output, "***");
        assert!(!debug_output.contains("my-password"));
    }

    #[test]
    fn display_is_redacted() {
        let secret = Secret::new("my-token".to_string());
        let display_output = format!("{}", secret);
        assert_eq!(display_output, "***");
        assert!(!display_output.contains("my-token"));
    }

    #[test]
    fn serialize_is_redacted() {
        let secret = Secret::new("super-secret".to_string());
        let json = serde_json::to_string(&secret).unwrap();
        assert_eq!(json, "\"***\"");
        assert!(!json.contains("super-secret"));
    }

    #[test]
    fn deserialize_unwraps_string() {
        let json = "\"actual-value\"";
        let secret: Secret<String> = serde_json::from_str(json).unwrap();
        assert_eq!(secret.expose(), "actual-value");
    }

    #[test]
    fn expose_returns_inner_value() {
        let secret = Secret::new("hidden".to_string());
        assert_eq!(secret.expose(), "hidden");
    }

    #[test]
    fn partial_eq_compares_inner_values() {
        let a = Secret::new("same".to_string());
        let b = Secret::new("same".to_string());
        let c = Secret::new("different".to_string());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn clone_preserves_inner_value() {
        let original = Secret::new("cloned".to_string());
        let cloned = original.clone();
        assert_eq!(original.expose(), cloned.expose());
    }
}
