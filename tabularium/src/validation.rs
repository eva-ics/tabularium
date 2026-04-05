//! Name rules for directories and documents — shared by library callers and servers.

use crate::{Error, Result};

/// Chat speaker id (`say` / WS `say`): non-empty, no line breaks or `:` (stored as an ATX `## id` heading).
pub fn validate_chat_speaker_id(id: impl AsRef<str>) -> Result<()> {
    let id = id.as_ref();
    if id.is_empty() {
        return Err(Error::InvalidInput("chat id must not be empty".into()));
    }
    if id.contains('\r') || id.contains('\n') {
        return Err(Error::InvalidInput(
            "chat id must not contain line breaks".into(),
        ));
    }
    if id.contains(':') {
        return Err(Error::InvalidInput("chat id must not contain ':'".into()));
    }
    Ok(())
}

/// Escape backslashes and `#` so a speaker id is safe inside a single-line ATX markdown heading.
pub(crate) fn escape_chat_heading_label(id: &str) -> String {
    let mut s = String::with_capacity(id.len());
    for ch in id.chars() {
        if matches!(ch, '\\' | '#') {
            s.push('\\');
        }
        s.push(ch);
    }
    s
}

/// Category and document names must not contain `/` or `\`, and must not be pure decimal strings.
pub fn validate_entity_name(name: impl AsRef<str>) -> Result<()> {
    let name = name.as_ref();
    if name.is_empty() {
        return Err(Error::InvalidInput("name must not be empty".into()));
    }
    if name.contains('/') {
        return Err(Error::InvalidInput("name must not contain '/'".into()));
    }
    if name.contains('\\') {
        return Err(Error::InvalidInput("name must not contain '\\'".into()));
    }
    if name.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::InvalidInput(
            "name must not be a pure decimal number".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{escape_chat_heading_label, validate_chat_speaker_id};

    #[test]
    fn chat_id_rejects_empty_colon_and_newlines() {
        assert!(validate_chat_speaker_id("").is_err());
        assert!(validate_chat_speaker_id("a:b").is_err());
        assert!(validate_chat_speaker_id("a\nb").is_err());
        assert!(validate_chat_speaker_id("ok").is_ok());
    }

    #[test]
    fn escape_chat_heading_escapes_hash_and_backslash() {
        assert_eq!(escape_chat_heading_label("a"), "a");
        assert_eq!(escape_chat_heading_label("a#b"), "a\\#b");
        assert_eq!(escape_chat_heading_label("a\\b"), "a\\\\b");
    }
}
