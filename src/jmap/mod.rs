//! Shared JMAP operations used by both the CLI commands and the TUI.
//!
//! Each submodule owns one JMAP capability area. Keeping these here means the
//! CLI and the TUI cannot drift apart in how they talk to the server.

pub mod calendar;
pub mod contacts;
pub mod mail;

/// Common result type for JMAP operations.
pub type JmapResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Extract the first error from a raw JMAP method response, if any.
///
/// Checks for top-level `error` responses and per-item failures in the given
/// `not_key` map (e.g. "notCreated", "notUpdated", "notDestroyed").
pub(crate) fn check_set_response(
    resp: &jmap_types::JmapResponse,
    method: &str,
    not_key: &str,
) -> JmapResult<()> {
    for (method_name, result, _call_id) in &resp.method_responses {
        if method_name == "error" {
            let error_type = result["type"].as_str().unwrap_or("unknown");
            let description = result["description"].as_str().unwrap_or("");
            return Err(format!("JMAP error: {error_type} — {description}").into());
        }
        if method_name == method {
            if let Some(failures) = result[not_key].as_object() {
                if let Some((key, err)) = failures.iter().next() {
                    let err_type = err["type"].as_str().unwrap_or("unknown");
                    let err_desc = err["description"].as_str().unwrap_or("");
                    return Err(
                        format!("{method} failed for {key}: {err_type} — {err_desc}").into(),
                    );
                }
            }
        }
    }
    Ok(())
}
