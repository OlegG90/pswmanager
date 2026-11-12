//! Why a command did not do what the window asked, as the window gets it.
//! Most failures are only shown, so a command's error is plain text; a
//! command with a failure its caller acts on (the user closed the prompt)
//! returns a [CommandError], which names that failure with a [Code]. Nothing
//! compares an error's text to tell which failure it is.

use crate::dbfile::OpenError;
use serde::Serialize;

/// A failure a caller acts on, as the window names it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Code {
    /// The user closed the prompt (Windows Hello's, the fingerprint's):
    /// nothing went wrong, and nothing is shown.
    Cancelled,
}

/// A command's error: the message to show, and which failure it is when the
/// caller may act on it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandError {
    code: Option<Code>,
    message: String,
}

impl CommandError {
    /// The user closed the prompt ([Code::Cancelled]).
    pub fn cancelled() -> Self {
        CommandError { code: Some(Code::Cancelled), message: "Cancelled".into() }
    }
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        CommandError { code: None, message }
    }
}

impl From<&str> for CommandError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

impl From<OpenError> for CommandError {
    fn from(error: OpenError) -> Self {
        error.to_string().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_gets_the_code_beside_the_message() {
        let cancelled = serde_json::to_value(CommandError::cancelled()).unwrap();
        assert_eq!(cancelled, serde_json::json!({ "code": "cancelled", "message": "Cancelled" }));
        let other = serde_json::to_value(CommandError::from("The database is locked")).unwrap();
        assert_eq!(other, serde_json::json!({ "code": null, "message": "The database is locked" }));
    }
}
