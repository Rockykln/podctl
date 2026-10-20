//! The shape of `--json`: one envelope for every command, one line per
//! object. Scripts read `schema` to know what they are looking at.

use serde::Serialize;

/// Bumped only when a field changes meaning or disappears. Adding one
/// does not need it.
pub const SCHEMA: u32 = 1;

#[derive(Serialize)]
pub struct Envelope<T: Serialize> {
    pub schema: u32,
    pub ok: bool,
    pub kind: &'static str,
    pub data: T,
}

impl<T: Serialize> Envelope<T> {
    pub fn ok(kind: &'static str, data: T) -> Self {
        Self {
            schema: SCHEMA,
            ok: true,
            kind,
            data,
        }
    }

    /// Compact and on one line, so `while read line` works and a stream
    /// of them is NDJSON.
    pub fn line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            // A serialiser that cannot write our own types is a bug, but
            // printing nothing at all would be worse than saying so.
            format!("{{\"schema\":{SCHEMA},\"ok\":false,\"kind\":\"err\",\"data\":{{\"message\":\"could not serialise the answer\"}}}}")
        })
    }

    pub fn pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| self.line())
    }

    pub fn print(&self, pretty: bool) {
        if pretty {
            println!("{}", self.pretty());
        } else {
            println!("{}", self.line());
        }
    }
}

#[derive(Serialize)]
pub struct ErrorData<'a> {
    pub message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<&'a str>,
}

/// The error envelope. `ok` is false and the exit code says more; the
/// message is the same text the terminal form prints.
pub fn error(message: &str) -> Envelope<ErrorData<'_>> {
    Envelope {
        schema: SCHEMA,
        ok: false,
        kind: "err",
        data: ErrorData {
            message,
            hint: None,
        },
    }
}

/// The same, with the `hint:` line a person would get.
pub fn error_with_hint<'a>(message: &'a str, hint: &'a str) -> Envelope<ErrorData<'a>> {
    Envelope {
        schema: SCHEMA,
        ok: false,
        kind: "err",
        data: ErrorData {
            message,
            hint: Some(hint),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Envelope, error};

    /// Key order is part of the contract: `schema` first, so a reader
    /// can tell at a glance what it got.
    #[test]
    fn the_envelope_has_a_fixed_shape() {
        let e = Envelope::ok("pong", serde_json::json!({}));
        assert_eq!(
            e.line(),
            r#"{"schema":1,"ok":true,"kind":"pong","data":{}}"#
        );
        assert_eq!(
            error("no AirPods paired").line(),
            r#"{"schema":1,"ok":false,"kind":"err","data":{"message":"no AirPods paired"}}"#
        );
        assert_eq!(
            super::error_with_hint("unknown command 'stauts'", "did you mean 'status'?").line(),
            r#"{"schema":1,"ok":false,"kind":"err","data":{"message":"unknown command 'stauts'","hint":"did you mean 'status'?"}}"#
        );
    }

    /// One line, always — a stream of envelopes is NDJSON.
    #[test]
    fn a_line_is_a_line() {
        let e = Envelope::ok("meter", serde_json::json!({"rms_dbfs": -12.5}));
        assert!(!e.line().contains('\n'));
        assert!(e.pretty().contains('\n'));
    }
}
