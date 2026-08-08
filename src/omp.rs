//! OMP (Oh My Pi) session transcript resolver and parser.
//!
//! Resolves OMP session `.jsonl` files from `$PI_CODING_AGENT_DIR/sessions` or
//! `$HOME/.omp/agent/sessions`, prioritizing candidate folders matching the
//! current working repository, and formats session logs into clean Markdown transcripts.

use std::path::{Path, PathBuf};
use crate::exit::ClipfError;

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        if let JsonValue::Object(kv) = self {
            kv.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        } else {
            None
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let JsonValue::String(s) = self {
            Some(s)
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn as_array(&self) -> Option<&Vec<JsonValue>> {
        if let JsonValue::Array(arr) = self {
            Some(arr)
        } else {
            None
        }
    }
}

pub fn parse_json(s: &str) -> Option<JsonValue> {
    let mut chars = s.chars().peekable();
    skip_ws(&mut chars);
    let val = parse_val(&mut chars)?;
    skip_ws(&mut chars);
    Some(val)
}

fn skip_ws<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) {
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else {
            break;
        }
    }
}

fn parse_val<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> Option<JsonValue> {
    skip_ws(chars);
    let &c = chars.peek()?;
    match c {
        '"' => parse_string(chars).map(JsonValue::String),
        '{' => parse_object(chars),
        '[' => parse_array(chars),
        't' | 'f' => parse_bool(chars),
        'n' => parse_null(chars),
        '-' | '0'..='9' => parse_number(chars),
        _ => None,
    }
}

fn parse_string<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> Option<String> {
    if chars.next()? != '"' {
        return None;
    }
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => {
                let escaped = chars.next()?;
                match escaped {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\x08'),
                    'f' => out.push('\x0c'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let mut hex = String::new();
                        for _ in 0..4 {
                            hex.push(chars.next()?);
                        }
                        if let Ok(code) = u32::from_str_radix(&hex, 16) {
                            if let Some(ch) = std::char::from_u32(code) {
                                out.push(ch);
                            }
                        }
                    }
                    _ => out.push(escaped),
                }
            }
            _ => out.push(c),
        }
    }
    None
}

fn parse_object<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> Option<JsonValue> {
    chars.next()?; // consume '{'
    let mut kv = Vec::new();
    loop {
        skip_ws(chars);
        let &c = chars.peek()?;
        if c == '}' {
            chars.next();
            return Some(JsonValue::Object(kv));
        }
        if c == '"' {
            let key = parse_string(chars)?;
            skip_ws(chars);
            if chars.next()? != ':' {
                return None;
            }
            let val = parse_val(chars)?;
            kv.push((key, val));
            skip_ws(chars);
            match chars.peek() {
                Some(',') => {
                    chars.next();
                }
                Some('}') => {
                    chars.next();
                    return Some(JsonValue::Object(kv));
                }
                _ => return None,
            }
        } else {
            return None;
        }
    }
}

fn parse_array<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> Option<JsonValue> {
    chars.next()?; // consume '['
    let mut items = Vec::new();
    loop {
        skip_ws(chars);
        let &c = chars.peek()?;
        if c == ']' {
            chars.next();
            return Some(JsonValue::Array(items));
        }
        let val = parse_val(chars)?;
        items.push(val);
        skip_ws(chars);
        match chars.peek() {
            Some(',') => {
                chars.next();
            }
            Some(']') => {
                chars.next();
                return Some(JsonValue::Array(items));
            }
            _ => return None,
        }
    }
}

fn parse_bool<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> Option<JsonValue> {
    let mut s = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphabetic() {
            s.push(chars.next()?);
        } else {
            break;
        }
    }
    match s.as_str() {
        "true" => Some(JsonValue::Bool(true)),
        "false" => Some(JsonValue::Bool(false)),
        _ => None,
    }
}

fn parse_null<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> Option<JsonValue> {
    let mut s = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphabetic() {
            s.push(chars.next()?);
        } else {
            break;
        }
    }
    if s == "null" {
        Some(JsonValue::Null)
    } else {
        None
    }
}

fn parse_number<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> Option<JsonValue> {
    let mut s = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E' {
            s.push(chars.next()?);
        } else {
            break;
        }
    }
    s.parse::<f64>().ok().map(JsonValue::Number)
}

fn extract_text_from_content(content: &JsonValue) -> String {
    match content {
        JsonValue::String(s) => s.clone(),
        JsonValue::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let JsonValue::Object(_) = item {
                    if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                        if !t.is_empty() {
                            parts.push(t);
                        }
                    }
                }
            }
            parts.join("\n")
        }
        _ => String::new(),
    }
}

pub fn read_session_transcript(path: &Path) -> Result<Vec<u8>, ClipfError> {
    let file = std::fs::File::open(path)
        .map_err(|e| ClipfError::input(format!("cannot open session file {}: {e}", path.display())))?;
    let reader = std::io::BufReader::new(file);
    use std::io::BufRead;

    let mut title: Option<String> = None;
    let mut messages: Vec<(String, String)> = Vec::new();

    for line in reader.lines() {
        let line = line.map_err(|e| ClipfError::input(format!("reading session file {}: {e}", path.display())))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(val) = parse_json(&line) {
            if title.is_none() {
                if let Some(t) = val.get("title").and_then(|v| v.as_str()) {
                    if !t.trim().is_empty() {
                        title = Some(t.to_string());
                    }
                }
            }
            if let Some(msg_type) = val.get("type").and_then(|v| v.as_str()) {
                if msg_type == "message" {
                    if let Some(msg) = val.get("message") {
                        if let Some(role) = msg.get("role").and_then(|v| v.as_str()) {
                            let normalized_role = match role {
                                "user" | "developer" => Some("User"),
                                "assistant" => Some("Assistant"),
                                _ => None,
                            };
                            if let Some(role_name) = normalized_role {
                                if let Some(content) = msg.get("content") {
                                    let text = extract_text_from_content(content);
                                    let trimmed = text.trim();
                                    if !trimmed.is_empty() {
                                        messages.push((role_name.to_string(), trimmed.to_string()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let header_title = title.unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "OMP Session Transcript".to_string())
    });

    let mut out = String::new();
    out.push_str("# ");
    out.push_str(&header_title);
    out.push_str("\n\n");

    for (role, text) in messages {
        out.push_str("## ");
        out.push_str(&role);
        out.push_str("\n\n");
        out.push_str(&text);
        out.push_str("\n\n");
        out.push_str("\n\n");
    }

    Ok(out.into_bytes())
}
pub fn read_session_raw_heredoc(path: &Path) -> Result<Vec<u8>, ClipfError> {
    let raw = std::fs::read(path)
        .map_err(|e| ClipfError::input(format!("cannot read session file {}: {e}", path.display())))?;
    let content_str = String::from_utf8_lossy(&raw);

    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();

    let parent_dir_name = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();

    let mut out = String::new();
    out.push_str(&format!(
        "mkdir -p ~/.omp/agent/sessions/{parent_dir_name} && cat << 'EOF_CLIPF_OMP' > ~/.omp/agent/sessions/{parent_dir_name}/{filename}\n"
    ));
    out.push_str(&content_str);
    if !content_str.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("EOF_CLIPF_OMP\n");

    Ok(out.into_bytes())
}

pub fn sessions_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        if !dir.is_empty() {
            let p = PathBuf::from(dir);
            if p.file_name().map_or(false, |n| n == "sessions") {
                return Some(p);
            } else {
                return Some(p.join("sessions"));
            }
        }
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok()?;
    Some(PathBuf::from(home).join(".omp").join("agent").join("sessions"))
}

pub fn resolve_session_path(session_query: &str) -> Result<PathBuf, ClipfError> {
    let base_dir = sessions_dir().ok_or_else(|| {
        ClipfError::input("could not determine OMP sessions directory ($HOME not set)")
    })?;

    if !base_dir.exists() || !base_dir.is_dir() {
        return Err(ClipfError::input(format!(
            "OMP sessions directory does not exist: {}",
            base_dir.display()
        )));
    }

    // Try candidate workspace folders first.
    let cwd = std::env::current_dir().ok();
    let cwd_basename = cwd.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy());

    let mut workspace_dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(dir_name) = path.file_name().map(|n| n.to_string_lossy()) {
                    if let Some(ref name) = cwd_basename {
                        if dir_name == format!("-{name}")
                            || dir_name.ends_with(&format!("-{name}"))
                            || dir_name.contains(name.as_ref())
                        {
                            workspace_dirs.push(path);
                        }
                    }
                }
            }
        }
    }

    // Try finding session in candidate workspace dirs first
    if !workspace_dirs.is_empty() {
        if let Some(found) = find_session_in_dirs(&workspace_dirs, session_query) {
            return Ok(found);
        }
    }

    // Fallback: search across all workspace subdirectories in base_dir
    let mut all_dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                all_dirs.push(path);
            }
        }
    }

    if let Some(found) = find_session_in_dirs(&all_dirs, session_query) {
        return Ok(found);
    }

    Err(ClipfError::input(format!(
        "no OMP session found matching '{session_query}'"
    )))
}

fn find_session_in_dirs(dirs: &[PathBuf], session_query: &str) -> Option<PathBuf> {
    let is_latest = session_query == "latest";
    let mut candidates: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

    for dir in dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().map_or(false, |ext| ext == "jsonl") {
                    let file_name = path.file_name().unwrap().to_string_lossy();
                    let matches = if is_latest {
                        true
                    } else {
                        file_name.contains(session_query)
                    };
                    if matches {
                        let mtime = std::fs::metadata(&path)
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        candidates.push((path, mtime));
                    }
                }
            }
        }
    }

    if candidates.is_empty() {
        None
    } else {
        // Sort by modification time descending
        candidates.sort_by(|a, b| b.1.cmp(&a.1));
        Some(candidates[0].0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_primitives() {
        assert_eq!(parse_json("true"), Some(JsonValue::Bool(true)));
        assert_eq!(parse_json("false"), Some(JsonValue::Bool(false)));
        assert_eq!(parse_json("null"), Some(JsonValue::Null));
        assert_eq!(parse_json("123.45"), Some(JsonValue::Number(123.45)));
        assert_eq!(parse_json("\"hello\\nworld\""), Some(JsonValue::String("hello\nworld".to_string())));
    }

    #[test]
    fn parse_json_object_and_array() {
        let json = r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#;
        let parsed = parse_json(json).unwrap();
        assert_eq!(parsed.get("type").unwrap().as_str(), Some("message"));
        let msg = parsed.get("message").unwrap();
        assert_eq!(msg.get("role").unwrap().as_str(), Some("user"));
        let content = msg.get("content").unwrap().as_array().unwrap();
        assert_eq!(content[0].get("text").unwrap().as_str(), Some("hello"));
    }

    #[test]
    fn read_transcript_from_jsonl() {
        let temp_dir = std::env::temp_dir().join("clipf_test_omp");
        let _ = std::fs::create_dir_all(&temp_dir);
        let session_file = temp_dir.join("test_session.jsonl");

        let jsonl = r#"{"type":"title","title":"Test Session"}
{"type":"message","message":{"role":"user","content":"Hello world"}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Hi there!"}]}}
"#;
        std::fs::write(&session_file, jsonl).unwrap();

        let bytes = read_session_transcript(&session_file).unwrap();
        let markdown = String::from_utf8(bytes).unwrap();

        assert!(markdown.starts_with("# Test Session\n\n"));
        assert!(markdown.contains("## User\n\nHello world\n\n"));
        assert!(markdown.contains("## Assistant\n\nHi there!\n\n"));

        let _ = std::fs::remove_file(&session_file);
        let _ = std::fs::remove_dir(&temp_dir);
    }
    #[test]
    fn resolve_session_path_with_env_dir() {
        let temp_dir = std::env::temp_dir().join("clipf_test_omp_dir");
        let sessions_dir = temp_dir.join("sessions");
        let ws_dir = sessions_dir.join("-clipf");
        let _ = std::fs::create_dir_all(&ws_dir);
        let session_file = ws_dir.join("2026-08-08T13-21-53-337Z_019fe189-82b9-7000-bf75-068024df70fa.jsonl");
        std::fs::write(&session_file, r#"{"type":"title","title":"Env Test"}"#).unwrap();

        std::env::set_var("PI_CODING_AGENT_DIR", temp_dir.to_str().unwrap());

        let resolved = resolve_session_path("latest").unwrap();
        assert_eq!(resolved, session_file);

        let resolved_prefix = resolve_session_path("019fe189").unwrap();
        assert_eq!(resolved_prefix, session_file);

        std::env::remove_var("PI_CODING_AGENT_DIR");
        let _ = std::fs::remove_file(&session_file);
        let _ = std::fs::remove_dir(&ws_dir);
        let _ = std::fs::remove_dir(&sessions_dir);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    #[test]
    fn read_raw_heredoc_from_jsonl() {
        let temp_dir = std::env::temp_dir().join("clipf_test_omp_heredoc");
        let ws_dir = temp_dir.join("-clipf");
        let _ = std::fs::create_dir_all(&ws_dir);
        let session_file = ws_dir.join("test_session.jsonl");

        let jsonl = "{\"type\":\"title\",\"title\":\"Test Session\"}\n";
        std::fs::write(&session_file, jsonl).unwrap();

        let bytes = read_session_raw_heredoc(&session_file).unwrap();
        let snippet = String::from_utf8(bytes).unwrap();

        assert!(snippet.contains("mkdir -p ~/.omp/agent/sessions/-clipf"));
        assert!(snippet.contains("cat << 'EOF_CLIPF_OMP' > ~/.omp/agent/sessions/-clipf/test_session.jsonl"));
        assert!(snippet.contains("{\"type\":\"title\",\"title\":\"Test Session\"}"));
        assert!(snippet.ends_with("EOF_CLIPF_OMP\n"));

        let _ = std::fs::remove_file(&session_file);
        let _ = std::fs::remove_dir(&ws_dir);
        let _ = std::fs::remove_dir(&temp_dir);
    }
}
