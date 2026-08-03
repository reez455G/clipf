//! A hand-rolled JSON writer — just enough to emit clipf's own `--json`
//! output. Flat objects, one level of nested objects, and arrays of strings
//! or of objects. Not a general serializer: no parser, no traits, no
//! arbitrary nesting depth. If a future `--json` shape needs more than
//! this, that's a signal to reconsider the shape, not to grow this module
//! into a serde replacement.
//!
//! The one real trap here: paths and environment variable values are not
//! guaranteed to be valid UTF-8, but JSON strings must be. `Value::from_path`
//! / `Value::from_os_str` go through `to_string_lossy()` *deliberately*,
//! before anything is escaped — never format a `Path`/`OsStr` with `{}` or
//! `{:?}` on the way into a `Value::Str`, both can panic or leak raw bytes.
//!
//! Nothing calls this module yet — it lands standalone in D2 so it can be
//! reviewed and tested on its own; `--json` (D3) is what actually consumes
//! it. The `allow` below is temporary and comes off once that lands.
#![allow(dead_code)]

use std::ffi::OsStr;
use std::path::Path;

/// A JSON value clipf can emit.
pub enum Value {
    Null,
    Bool(bool),
    UInt(u64),
    Str(String),
    /// An array of values — used for arrays of strings and arrays of
    /// objects; nothing in this crate's schemas needs mixed-type arrays.
    Array(Vec<Value>),
    /// An object. Keep entries at most one level deep (an `Object`'s values
    /// may themselves be an `Object`, but that inner `Object`'s values
    /// should be scalars/arrays, not a further `Object`) — this module
    /// doesn't enforce that in the type system, callers just need to keep
    /// clipf's actual schemas that shallow.
    Object(Vec<(&'static str, Value)>),
}

impl Value {
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    /// Lossy by design: a non-UTF-8 path becomes valid UTF-8 with U+FFFD
    /// standing in for whatever couldn't be decoded, so the JSON output is
    /// always well-formed even for filenames the OS itself doesn't require
    /// to be UTF-8.
    pub fn from_path(p: &Path) -> Value {
        Value::Str(p.to_string_lossy().into_owned())
    }

    /// Same treatment for environment variable values, which POSIX also
    /// does not guarantee are valid UTF-8.
    pub fn from_os_str(s: &OsStr) -> Value {
        Value::Str(s.to_string_lossy().into_owned())
    }

    pub fn str_array<I, S>(items: I) -> Value
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Value::Array(items.into_iter().map(|s| Value::Str(s.into())).collect())
    }

    pub fn obj(entries: Vec<(&'static str, Value)>) -> Value {
        Value::Object(entries)
    }
}

/// Serialize a `Value` to a compact JSON string.
pub fn write(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::UInt(n) => write_uint(out, *n),
        Value::Str(s) => escape_into(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::Object(entries) => {
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                escape_into(out, k);
                out.push(':');
                write_value(out, v);
            }
            out.push('}');
        }
    }
}

fn write_uint(out: &mut String, mut n: u64) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    // Every byte just written is one of b'0'..=b'9', so this is always ASCII.
    out.push_str(std::str::from_utf8(&buf[i..]).expect("digits are always ASCII"));
}

fn escape_into(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let n = c as u32;
                out.push_str("\\u00");
                out.push(hex_digit(((n >> 4) & 0xf) as u8));
                out.push(hex_digit((n & 0xf) as u8));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'a' + (n - 10)) as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars() {
        assert_eq!(write(&Value::Null), "null");
        assert_eq!(write(&Value::Bool(true)), "true");
        assert_eq!(write(&Value::Bool(false)), "false");
        assert_eq!(write(&Value::UInt(0)), "0");
        assert_eq!(write(&Value::UInt(65536)), "65536");
        assert_eq!(write(&Value::UInt(u64::MAX)), u64::MAX.to_string());
    }

    #[test]
    fn quote_and_backslash_are_escaped() {
        assert_eq!(write(&Value::str("a\"b")), "\"a\\\"b\"");
        assert_eq!(write(&Value::str("a\\b")), "\"a\\\\b\"");
    }

    #[test]
    fn shorthand_control_escapes() {
        assert_eq!(write(&Value::str("\n")), "\"\\n\"");
        assert_eq!(write(&Value::str("\r")), "\"\\r\"");
        assert_eq!(write(&Value::str("\t")), "\"\\t\"");
        assert_eq!(write(&Value::str("\u{08}")), "\"\\b\"");
        assert_eq!(write(&Value::str("\u{0C}")), "\"\\f\"");
    }

    #[test]
    fn other_control_chars_use_u00xx() {
        assert_eq!(write(&Value::str("\u{01}")), "\"\\u0001\"");
        assert_eq!(write(&Value::str("\u{1f}")), "\"\\u001f\"");
        assert_eq!(write(&Value::str("\u{00}")), "\"\\u0000\"");
    }

    #[test]
    fn printable_non_ascii_passes_through_unescaped() {
        assert_eq!(write(&Value::str("héllo ☃ 世界")), "\"héllo ☃ 世界\"");
    }

    #[test]
    fn no_trailing_comma_arrays() {
        assert_eq!(write(&Value::Array(vec![])), "[]");
        assert_eq!(write(&Value::Array(vec![Value::str("a")])), "[\"a\"]");
        assert_eq!(
            write(&Value::Array(vec![Value::str("a"), Value::str("b")])),
            "[\"a\",\"b\"]"
        );
    }

    #[test]
    fn no_trailing_comma_objects() {
        assert_eq!(write(&Value::Object(vec![])), "{}");
        assert_eq!(
            write(&Value::Object(vec![("a", Value::Bool(true))])),
            "{\"a\":true}"
        );
        assert_eq!(
            write(&Value::Object(vec![
                ("a", Value::Bool(true)),
                ("b", Value::UInt(1))
            ])),
            "{\"a\":true,\"b\":1}"
        );
    }

    #[test]
    fn nested_object_one_level_and_array_of_objects() {
        let v = Value::Object(vec![
            (
                "backend",
                Value::obj(vec![
                    ("selected", Value::str("osc52")),
                    ("available", Value::str_array(["osc52"])),
                ]),
            ),
            (
                "warnings",
                Value::Array(vec![Value::obj(vec![
                    ("code", Value::str("x")),
                    ("message", Value::str("y")),
                ])]),
            ),
        ]);
        assert_eq!(
            write(&v),
            "{\"backend\":{\"selected\":\"osc52\",\"available\":[\"osc52\"]},\
             \"warnings\":[{\"code\":\"x\",\"message\":\"y\"}]}"
        );
    }

    #[test]
    fn lossy_path_round_trips_as_valid_json() {
        // json.rs deliberately has no parser to validate against, so "valid
        // JSON" here is checked by exact match against hand-written,
        // manifestly well-formed JSON (balanced quotes, correct escaping) -
        // not by round-tripping through a parser.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            // 0xFF is never valid UTF-8 on its own.
            let raw = OsStr::from_bytes(b"bad\xffname.txt");
            let path = Path::new(raw);
            let value = Value::obj(vec![("path", Value::from_path(path))]);
            let out = write(&value);
            assert!(out.contains('\u{FFFD}'), "expected a replacement char in {out:?}");
            assert_eq!(out, "{\"path\":\"bad\u{FFFD}name.txt\"}");
        }
    }
}
