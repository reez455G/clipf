//! clipf - copy file contents to the clipboard, locally or over SSH.

mod backend;
mod base64;
mod check;
mod cli;
mod exit;
mod json;
mod osc52;
mod secret;
mod term;

use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use backend::Backend;
use cli::{Action, Config};
use exit::{ClipfError, EXIT_OK};
use json::Value;
use secret::{Secret, SecretString};
use term::Sink;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let cfg = match cli::parse(args) {
        Ok(c) => c,
        // Every cli::parse error is a usage error: unknown flag, missing or
        // malformed flag value, or conflicting flags.
        Err(e) => return fail(&ClipfError::usage(e)),
    };

    match cfg.action {
        Action::Help => {
            print!("{}", cli::help());
            ExitCode::from(EXIT_OK)
        }
        Action::Version => {
            println!("clipf {}", cli::VERSION);
            ExitCode::from(EXIT_OK)
        }
        Action::Check => {
            if cfg.json {
                println!("{}", check::report_json(&cfg));
            } else {
                print!("{}", check::report(&cfg));
            }
            ExitCode::from(EXIT_OK)
        }
        // cli::parse rejects --json with -O/--paste, so run_paste never
        // needs to know about cfg.json.
        Action::Paste => match run_paste(&cfg) {
            Ok(()) => ExitCode::from(EXIT_OK),
            Err(e) => fail(&e),
        },
        Action::Copy => {
            let source = source_label(&cfg);
            let result = run_copy(&cfg);
            if cfg.json {
                println!("{}", copy_report_json(&cfg, &source, &result));
            }
            match result {
                Ok(_) => ExitCode::from(EXIT_OK),
                Err(e) => fail(&e),
            }
        }
    }
}

fn fail(err: &ClipfError) -> ExitCode {
    eprintln!("clipf: {err}");
    ExitCode::from(err.code())
}

fn note(msg: &str) {
    eprintln!("clipf: {msg}");
}

/// The `source` field `--json` reports: known before (and independent of)
/// whether the read actually succeeds, so it's available for both the
/// success and the failure JSON shape.
fn source_label(cfg: &Config) -> String {
    match &cfg.file {
        Some(p) => p.display().to_string(),
        None => "<stdin>".to_string(),
    }
}

fn run_paste(cfg: &Config) -> Result<(), ClipfError> {
    let b = cfg.backend.unwrap_or_else(Backend::detect);
    let data = b.paste()?;
    io::stdout()
        .write_all(&data)
        .map_err(|e| ClipfError::backend_failed(format!("writing to stdout: {e}")))
}

/// What a successful copy did, for `--json` to report.
struct CopyOutcome {
    bytes: usize,
    backend: &'static str,
}

fn run_copy(cfg: &Config) -> Result<CopyOutcome, ClipfError> {
    let (mut data, source) = read_input(cfg.file.as_deref())?;

    if cfg.strip_newline {
        data.trim_trailing_newlines();
    }
    let bytes = data.len();
    if data.is_empty() {
        note("input is empty - clearing the clipboard");
    }

    let chosen = cfg.backend.unwrap_or_else(Backend::detect);
    if cfg.verbose {
        note(&format!(
            "source={source} bytes={bytes} backend={}",
            chosen.name()
        ));
    }

    if chosen == Backend::Osc52 {
        copy_osc52(cfg, &data, &source)?;
    } else {
        if let Some(cmd) = chosen.command() {
            if backend::which(cmd).is_none() {
                return Err(ClipfError::backend_unavailable(format!(
                    "backend '{}' was requested but {cmd} is not installed",
                    chosen.name()
                )));
            }
        }
        if cfg.dry_run {
            note(&format!(
                "dry-run: would pipe {bytes} bytes to {}",
                chosen.name()
            ));
        } else {
            chosen.copy_local(&data)?;
        }
    }

    if cfg.tee {
        io::stdout()
            .write_all(&data)
            .map_err(|e| ClipfError::backend_failed(format!("writing to stdout: {e}")))?;
    }

    if cfg.verbose && !cfg.dry_run {
        note(&format!("copied {bytes} bytes via {}", chosen.name()));
    }

    Ok(CopyOutcome {
        bytes,
        backend: chosen.name(),
    })
}

fn copy_osc52(cfg: &Config, data: &[u8], source: &str) -> Result<(), ClipfError> {
    let bytes = data.len();

    if cfg.max_bytes > 0 && bytes > cfg.max_bytes && !cfg.force {
        let encoded = base64::encoded_len(bytes);
        note(&format!(
            "{bytes} bytes ({encoded} once base64-encoded) exceeds the OSC 52 \
             guard of {} bytes.",
            cfg.max_bytes
        ));
        note("Terminals truncate oversized payloads without reporting an error,");
        note("so this would most likely copy a partial file. Options:");
        note(&format!(
            "  - run from your local shell:  ssh HOST 'cat {source}' | clipf"
        ));
        note(&format!(
            "  - raise the cap:              clipf --max 0 --force {source}"
        ));
        return Err(ClipfError::too_big(
            format!(
                "{bytes} bytes ({encoded} once base64-encoded) exceeds the OSC 52 guard \
                 of {} bytes",
                cfg.max_bytes
            ),
            bytes,
        ));
    }

    let b64 = SecretString(base64::encode(data));
    let mux = term::multiplexer();
    let seq = osc52::build(&b64, mux, cfg.passthrough);
    let mut sink = Sink::open();

    if cfg.dry_run {
        note(&format!(
            "dry-run: would write a {}-byte escape sequence to {}",
            seq.len(),
            sink.name()
        ));
        return Ok(());
    }

    if !sink.is_tty() && cfg.verbose {
        note("no controlling terminal; writing the escape sequence to stderr");
    }

    sink.write_all(&seq.bytes)
        .map_err(|e| ClipfError::backend_failed(format!("writing to {}: {e}", sink.name())))?;

    Ok(())
}

/// Read the payload into memory. Never staged on disk: the whole point is that
/// this often carries credentials.
fn read_input(file: Option<&Path>) -> Result<(Secret, String), ClipfError> {
    match file {
        None => {
            if io::stdin().is_terminal() {
                return Err(ClipfError::usage(
                    "no FILE given and stdin is a terminal (try --help)",
                ));
            }
            let mut buf = Secret::with_capacity(8192);
            io::stdin()
                .lock()
                .read_to_end(buf.as_mut_vec())
                .map_err(|e| ClipfError::input(format!("reading stdin: {e}")))?;
            Ok((buf, "<stdin>".to_string()))
        }
        Some(p) => {
            let md = std::fs::metadata(p).map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => {
                    ClipfError::input(format!("no such file: {}", p.display()))
                }
                io::ErrorKind::PermissionDenied => ClipfError::input(format!(
                    "cannot read: {} (permission denied)",
                    p.display()
                )),
                _ => ClipfError::input(format!("cannot stat {}: {e}", p.display())),
            })?;
            if md.is_dir() {
                return Err(ClipfError::input(format!("{} is a directory", p.display())));
            }

            // Preallocate the exact length so the buffer never reallocates and
            // leaves an unwiped copy of the data behind.
            let mut buf = Secret::with_capacity(md.len() as usize + 1);
            let mut f = std::fs::File::open(p).map_err(|e| match e.kind() {
                io::ErrorKind::PermissionDenied => ClipfError::input(format!(
                    "cannot read: {} (permission denied)",
                    p.display()
                )),
                _ => ClipfError::input(format!("cannot open {}: {e}", p.display())),
            })?;
            f.read_to_end(buf.as_mut_vec())
                .map_err(|e| ClipfError::input(format!("reading {}: {e}", p.display())))?;
            Ok((buf, p.display().to_string()))
        }
    }
}

/// The `--json` form of a copy attempt's outcome. `bytes`/`encoded_bytes`/
/// `backend` are `null` when execution failed before they were known —
/// except for `TooBig`, which carries `bytes` on the error itself (see
/// `ClipfError::too_big`) because that's exactly the refusal where knowing
/// the size is most useful.
fn copy_report_json(
    cfg: &Config,
    source: &str,
    result: &Result<CopyOutcome, ClipfError>,
) -> String {
    let (bytes, backend, error): (Option<usize>, Option<&'static str>, Option<&ClipfError>) =
        match result {
            Ok(outcome) => (Some(outcome.bytes), Some(outcome.backend), None),
            Err(e) if e.kind == exit::ErrorKind::TooBig => {
                (e.bytes, Some(Backend::Osc52.name()), Some(e))
            }
            Err(e) => (None, None, Some(e)),
        };

    let encoded_bytes = match backend {
        Some(b) if b == Backend::Osc52.name() => bytes.map(base64::encoded_len),
        _ => None,
    };

    let mut entries = vec![
        ("schema", Value::UInt(1)),
        ("clipf", Value::str(cli::VERSION)),
        ("ok", Value::Bool(result.is_ok())),
        ("source", Value::str(source)),
        (
            "bytes",
            bytes.map_or(Value::Null, |n| Value::UInt(n as u64)),
        ),
        (
            "encoded_bytes",
            encoded_bytes.map_or(Value::Null, |n| Value::UInt(n as u64)),
        ),
        ("backend", backend.map_or(Value::Null, Value::str)),
        ("stripped_newline", Value::Bool(cfg.strip_newline)),
        ("dry_run", Value::Bool(cfg.dry_run)),
    ];

    if let Some(e) = error {
        entries.push((
            "error",
            Value::obj(vec![
                ("code", Value::UInt(e.code() as u64)),
                ("kind", Value::str(e.kind.json_name())),
                ("message", Value::str(e.message.clone())),
            ]),
        ));
    }

    json::write(&Value::Object(entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_input_error() {
        // read_input's Ok variant carries a Secret, which deliberately has
        // no Debug impl (avoids ever formatting payload bytes) — so this
        // matches explicitly instead of calling unwrap_err().
        match read_input(Some(Path::new("/definitely/does/not/exist/clipf-xyz"))) {
            Err(e) => assert_eq!(e.code(), 4),
            Ok(_) => panic!("expected a missing-file error"),
        }
    }

    #[test]
    fn directory_is_input_error() {
        let dir = std::env::temp_dir();
        match read_input(Some(&dir)) {
            Err(e) => assert_eq!(e.code(), 4),
            Ok(_) => panic!("expected a directory error"),
        }
    }

    #[test]
    fn unknown_flag_wrapped_as_usage_error() {
        // Mirrors exactly what main() does with a cli::parse error.
        let parse_err = cli::parse(["--nonexistent".to_string()]).unwrap_err();
        assert_eq!(ClipfError::usage(parse_err).code(), 1);
    }

    #[test]
    fn oversized_payload_without_force_returns_too_big() {
        let cfg = Config {
            max_bytes: 10,
            ..Config::default()
        };
        let data = vec![0u8; 100];
        let err = copy_osc52(&cfg, &data, "test").unwrap_err();
        assert_eq!(err.code(), 3);
        assert_eq!(err.bytes, Some(100));
    }

    #[test]
    fn paste_over_osc52_is_unsupported() {
        let cfg = Config {
            backend: Some(Backend::Osc52),
            ..Config::default()
        };
        let err = run_paste(&cfg).unwrap_err();
        assert_eq!(err.code(), 8);
    }

    #[test]
    fn copy_report_json_success_shape() {
        let cfg = Config::default();
        let result: Result<CopyOutcome, ClipfError> = Ok(CopyOutcome {
            bytes: 1234,
            backend: "osc52",
        });
        let out = copy_report_json(&cfg, "server.conf", &result);
        assert!(out.contains("\"ok\":true"));
        assert!(out.contains("\"source\":\"server.conf\""));
        assert!(out.contains("\"bytes\":1234"));
        assert!(out.contains("\"backend\":\"osc52\""));
        assert!(out.contains(&format!(
            "\"encoded_bytes\":{}",
            base64::encoded_len(1234)
        )));
        assert!(!out.contains("\"error\""));
        assert!(!out.contains(",}"));
        assert!(!out.contains(",]"));
    }

    #[test]
    fn copy_report_json_non_osc52_backend_has_no_encoded_bytes() {
        let cfg = Config::default();
        let result: Result<CopyOutcome, ClipfError> = Ok(CopyOutcome {
            bytes: 42,
            backend: "xclip",
        });
        let out = copy_report_json(&cfg, "f.txt", &result);
        assert!(out.contains("\"encoded_bytes\":null"));
    }

    #[test]
    fn copy_report_json_too_big_keeps_bytes_and_backend() {
        let cfg = Config::default();
        let result: Result<CopyOutcome, ClipfError> =
            Err(ClipfError::too_big("100 bytes exceeds the guard", 100));
        let out = copy_report_json(&cfg, "-", &result);
        assert!(out.contains("\"ok\":false"));
        assert!(out.contains("\"bytes\":100"));
        assert!(out.contains("\"backend\":\"osc52\""));
        assert!(out.contains("\"error\":{"));
        assert!(out.contains("\"code\":3"));
        assert!(out.contains("\"kind\":\"too_big\""));
    }

    #[test]
    fn copy_report_json_input_error_has_null_bytes_and_backend() {
        let cfg = Config::default();
        let result: Result<CopyOutcome, ClipfError> =
            Err(ClipfError::input("no such file: x.txt"));
        let out = copy_report_json(&cfg, "x.txt", &result);
        assert!(out.contains("\"ok\":false"));
        assert!(out.contains("\"bytes\":null"));
        assert!(out.contains("\"backend\":null"));
        assert!(out.contains("\"kind\":\"input\""));
        assert!(out.contains("\"code\":4"));
    }

    #[test]
    fn source_label_matches_file_or_stdin() {
        let cfg = Config {
            file: Some(std::path::PathBuf::from("a.txt")),
            ..Config::default()
        };
        assert_eq!(source_label(&cfg), "a.txt");
        assert_eq!(source_label(&Config::default()), "<stdin>");
    }
}
