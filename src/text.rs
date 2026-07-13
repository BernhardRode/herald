//! Text display helpers shared by every output path.
//!
//! All CLI output paths must call [`sanitize_display`] before printing
//! server-controlled content to prevent ANSI escape injection and other
//! control-character attacks.

/// Strip ANSI escape sequences and C0/C1 control characters from a string.
///
/// Preserves only:
/// - TAB (0x09)
/// - LF (0x0A)
/// - Printable characters
///
/// All other characters — including ESC (0x1B), DEL (0x7F), and the C1 range
/// (U+0080–U+009F, which contains the single-char CSI U+009B) — are removed.
pub fn sanitize_display(input: &str) -> String {
    input
        .chars()
        .filter(|&c| c == '\t' || c == '\n' || (c >= ' ' && c != '\x7F' && !is_c1_control(c)))
        .collect()
}

fn is_c1_control(c: char) -> bool {
    ('\u{80}'..='\u{9F}').contains(&c)
}

/// Truncate a string to `max_len` characters, appending "…" if truncated.
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let mut truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        truncated.push('…');
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_plain_text() {
        let input = "Hello, world!";
        assert_eq!(sanitize_display(input), "Hello, world!");
    }

    #[test]
    fn preserves_tab_and_newline() {
        let input = "line1\n\tindented";
        assert_eq!(sanitize_display(input), "line1\n\tindented");
    }

    #[test]
    fn strips_ansi_escape() {
        let input = "\x1B[31mred text\x1B[0m";
        assert_eq!(sanitize_display(input), "[31mred text[0m");
    }

    #[test]
    fn strips_null_and_other_c0_controls() {
        // NUL, SOH, STX, BEL, BS, VT, FF, CR, and other C0 chars
        let input = "\x00\x01\x02\x07\x08\x0B\x0C\x0D\x0Evisible";
        assert_eq!(sanitize_display(input), "visible");
    }

    #[test]
    fn strips_c1_controls() {
        // U+009B is a single-character CSI on some terminals — as dangerous as ESC [
        let input = "safe\u{9B}31mred\u{85}text";
        assert_eq!(sanitize_display(input), "safe31mredtext");
    }

    #[test]
    fn strips_del() {
        let input = "before\x7Fafter";
        assert_eq!(sanitize_display(input), "beforeafter");
    }

    #[test]
    fn preserves_unicode() {
        let input = "café ☕ 日本語";
        assert_eq!(sanitize_display(input), "café ☕ 日本語");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(sanitize_display(""), "");
    }

    #[test]
    fn all_control_chars_stripped_yields_empty() {
        let input = "\x00\x01\x02\x03\x04\x05\x06\x07\x08\x0B\x0C\x0D\x0E\x0F\x1B\x7F";
        assert_eq!(sanitize_display(input), "");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_adds_ellipsis() {
        assert_eq!(truncate_str("hello world", 6), "hello…");
    }

    #[test]
    fn truncate_counts_chars_not_bytes() {
        // 5 multi-byte chars must not be truncated at max_len 5
        assert_eq!(truncate_str("ééééé", 5), "ééééé");
    }
}
