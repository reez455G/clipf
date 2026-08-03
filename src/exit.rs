//! Exit codes and the error type that carries one.
//!
//! `ClipfError` pairs a human-readable message (always printed to stderr as
//! `clipf: <message>`) with the process exit code an agent or script can
//! branch on. This module is the only place the exit-code table is allowed
//! to drift from — see README.md's "Exit codes" section, which must be kept
//! in sync with `ErrorKind::code`.
//!
//! Code `2` is skipped deliberately (conventional shell "misuse of
//! builtin"). Code `7` (no controlling terminal for the OSC 52 path) is
//! reserved but currently unreachable: `term::Sink::open` never fails
//! outright, it falls back to writing the escape sequence to stderr, which
//! is the documented, deliberate behaviour for headless/CI/SSH sessions
//! with no tty. Turning that into a hard failure would break the "works
//! over a bare SSH pipe with nothing installed" guarantee the OSC 52 path
//! exists for, so no code path constructs `ErrorKind::NoTty` today.

use std::fmt;

pub const EXIT_OK: u8 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Unknown flag, a missing or malformed flag value, conflicting flags,
    /// or an invocation with nothing to read (no FILE, stdin is a
    /// terminal).
    Usage,
    /// The payload exceeds the OSC 52 size guard and `--force` was not
    /// given. Not really a "failure" so much as a refusal, but it shares
    /// the same exit-code/message shape as every other non-zero outcome,
    /// so it lives here too rather than as a separate `Ok(u8)` case.
    TooBig,
    /// The named file is missing, is a directory, permission was denied, or
    /// a read failed.
    Input,
    /// The requested backend's helper binary is not on PATH, could not be
    /// spawned at all, or (for `Osc52`, which has no local helper) was
    /// asked to run one anyway.
    BackendUnavailable,
    /// A backend helper spawned but exited non-zero, or a write into the
    /// copy/paste pipeline failed — the helper's stdin, our own stdout, or
    /// the OSC 52 sink.
    BackendFailed,
    // Reserved code path, see module doc — never constructed today.
    #[allow(dead_code)]
    /// Reserved: see the module doc. Never constructed today.
    NoTty,
    /// `--paste` against a backend that cannot be read back (OSC 52; most
    /// terminals disable the query form as a security measure).
    PasteUnsupported,
}

impl ErrorKind {
    pub const fn code(self) -> u8 {
        match self {
            ErrorKind::Usage => 1,
            ErrorKind::TooBig => 3,
            ErrorKind::Input => 4,
            ErrorKind::BackendUnavailable => 5,
            ErrorKind::BackendFailed => 6,
            ErrorKind::NoTty => 7,
            ErrorKind::PasteUnsupported => 8,
        }
    }

    /// Machine-stable name used as `error.kind` in `--json` output (see
    /// `src/json.rs`). Agents should branch on this or on `code`; the
    /// prose in `message` may be reworded across versions.
    pub fn json_name(self) -> &'static str {
        match self {
            ErrorKind::Usage => "usage",
            ErrorKind::TooBig => "too_big",
            ErrorKind::Input => "input",
            ErrorKind::BackendUnavailable => "backend_unavailable",
            ErrorKind::BackendFailed => "backend_failed",
            ErrorKind::NoTty => "no_tty",
            ErrorKind::PasteUnsupported => "paste_unsupported",
        }
    }
}

/// An error that already knows which exit code it must produce.
#[derive(Debug, Clone)]
pub struct ClipfError {
    pub kind: ErrorKind,
    pub message: String,
    /// The payload size, when known at the point of failure. Only
    /// `TooBig` sets this — `--json` (D3) reports `bytes`/`encoded_bytes`
    /// for that case even though the copy was refused, since that's
    /// exactly when knowing the size is most useful.
    pub bytes: Option<usize>,
}

impl ClipfError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            bytes: None,
        }
    }

    pub fn code(&self) -> u8 {
        self.kind.code()
    }
}

impl fmt::Display for ClipfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ClipfError {}

// Short constructors so call sites read as tersely as the old
// `Err(format!(...))` did: `Err(ClipfError::input(format!(...)))`.
macro_rules! ctor {
    ($name:ident, $kind:ident) => {
        impl ClipfError {
            pub fn $name(message: impl Into<String>) -> Self {
                Self::new(ErrorKind::$kind, message)
            }
        }
    };
}
ctor!(usage, Usage);
ctor!(input, Input);
ctor!(backend_unavailable, BackendUnavailable);
ctor!(backend_failed, BackendFailed);
ctor!(paste_unsupported, PasteUnsupported);

impl ClipfError {
    /// Unlike the other constructors, this one carries the payload size
    /// that triggered the refusal.
    pub fn too_big(message: impl Into<String>, bytes: usize) -> Self {
        Self {
            kind: ErrorKind::TooBig,
            message: message.into(),
            bytes: Some(bytes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_table_matches_the_readme() {
        assert_eq!(ErrorKind::Usage.code(), 1);
        assert_eq!(ErrorKind::TooBig.code(), 3);
        assert_eq!(ErrorKind::Input.code(), 4);
        assert_eq!(ErrorKind::BackendUnavailable.code(), 5);
        assert_eq!(ErrorKind::BackendFailed.code(), 6);
        assert_eq!(ErrorKind::NoTty.code(), 7);
        assert_eq!(ErrorKind::PasteUnsupported.code(), 8);
    }

    #[test]
    fn json_names_are_snake_case_and_stable() {
        for (kind, name) in [
            (ErrorKind::Usage, "usage"),
            (ErrorKind::TooBig, "too_big"),
            (ErrorKind::Input, "input"),
            (ErrorKind::BackendUnavailable, "backend_unavailable"),
            (ErrorKind::BackendFailed, "backend_failed"),
            (ErrorKind::NoTty, "no_tty"),
            (ErrorKind::PasteUnsupported, "paste_unsupported"),
        ] {
            assert_eq!(kind.json_name(), name);
        }
    }

    #[test]
    fn display_is_the_bare_message_with_no_added_prefix() {
        let e = ClipfError::input("no such file: x.txt");
        assert_eq!(e.to_string(), "no such file: x.txt");
        assert_eq!(e.code(), 4);
        assert_eq!(e.bytes, None);
    }

    #[test]
    fn too_big_carries_the_byte_count() {
        let e = ClipfError::too_big("134 bytes exceeds the guard", 134);
        assert_eq!(e.code(), 3);
        assert_eq!(e.bytes, Some(134));
    }
}
