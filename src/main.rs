//! clipf - copy file contents to the clipboard, locally or over SSH.

mod backend;
mod base64;
mod check;
mod cli;
mod osc52;
mod secret;
mod term;

use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use backend::Backend;
use cli::{Action, Config};
use secret::{Secret, SecretString};
use term::Sink;

const EXIT_OK: u8 = 0;
const EXIT_ERR: u8 = 1;
const EXIT_TOO_BIG: u8 = 3;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let cfg = match cli::parse(args) {
        Ok(c) => c,
        Err(e) => return fail(&e),
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
            print!("{}", check::report(&cfg));
            ExitCode::from(EXIT_OK)
        }
        Action::Paste => match run_paste(&cfg) {
            Ok(()) => ExitCode::from(EXIT_OK),
            Err(e) => fail(&e),
        },
        Action::Copy => match run_copy(&cfg) {
            Ok(code) => ExitCode::from(code),
            Err(e) => fail(&e),
        },
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("clipf: {msg}");
    ExitCode::from(EXIT_ERR)
}

fn note(msg: &str) {
    eprintln!("clipf: {msg}");
}

fn run_paste(cfg: &Config) -> Result<(), String> {
    let b = cfg.backend.unwrap_or_else(Backend::detect);
    let data = b.paste()?;
    io::stdout()
        .write_all(&data)
        .map_err(|e| format!("writing to stdout: {e}"))
}

fn run_copy(cfg: &Config) -> Result<u8, String> {
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
        let code = copy_osc52(cfg, &data, &source)?;
        if code != EXIT_OK {
            return Ok(code);
        }
    } else {
        if let Some(cmd) = chosen.command() {
            if backend::which(cmd).is_none() {
                return Err(format!(
                    "backend '{}' was requested but {cmd} is not installed",
                    chosen.name()
                ));
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
            .map_err(|e| format!("writing to stdout: {e}"))?;
    }

    if cfg.verbose && !cfg.dry_run {
        note(&format!("copied {bytes} bytes via {}", chosen.name()));
    }

    Ok(EXIT_OK)
}

fn copy_osc52(cfg: &Config, data: &[u8], source: &str) -> Result<u8, String> {
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
        return Ok(EXIT_TOO_BIG);
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
        return Ok(EXIT_OK);
    }

    if !sink.is_tty() && cfg.verbose {
        note("no controlling terminal; writing the escape sequence to stderr");
    }

    sink.write_all(&seq.bytes)
        .map_err(|e| format!("writing to {}: {e}", sink.name()))?;

    Ok(EXIT_OK)
}

/// Read the payload into memory. Never staged on disk: the whole point is that
/// this often carries credentials.
fn read_input(file: Option<&Path>) -> Result<(Secret, String), String> {
    match file {
        None => {
            if io::stdin().is_terminal() {
                return Err("no FILE given and stdin is a terminal (try --help)".into());
            }
            let mut buf = Secret::with_capacity(8192);
            io::stdin()
                .lock()
                .read_to_end(buf.as_mut_vec())
                .map_err(|e| format!("reading stdin: {e}"))?;
            Ok((buf, "<stdin>".to_string()))
        }
        Some(p) => {
            let md = std::fs::metadata(p).map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => format!("no such file: {}", p.display()),
                io::ErrorKind::PermissionDenied => {
                    format!("cannot read: {} (permission denied)", p.display())
                }
                _ => format!("cannot stat {}: {e}", p.display()),
            })?;
            if md.is_dir() {
                return Err(format!("{} is a directory", p.display()));
            }

            // Preallocate the exact length so the buffer never reallocates and
            // leaves an unwiped copy of the data behind.
            let mut buf = Secret::with_capacity(md.len() as usize + 1);
            let mut f = std::fs::File::open(p).map_err(|e| match e.kind() {
                io::ErrorKind::PermissionDenied => {
                    format!("cannot read: {} (permission denied)", p.display())
                }
                _ => format!("cannot open {}: {e}", p.display()),
            })?;
            f.read_to_end(buf.as_mut_vec())
                .map_err(|e| format!("reading {}: {e}", p.display()))?;
            Ok((buf, p.display().to_string()))
        }
    }
}
