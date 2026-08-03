//! `--check`: answer "why isn't this working?" without a round of guessing.

use std::env;
use std::fmt::Write as _;

use crate::backend::{which, Backend};
use crate::cli::{BackendSource, Config, VERSION};
use crate::json::{self, Value};
use crate::term::{self, Multiplexer, Osc52Support, Sink};

pub fn report(cfg: &Config) -> String {
    let mut o = String::new();
    let sink = Sink::open();
    let mux = term::multiplexer();
    let chosen = cfg.backend.unwrap_or_else(Backend::detect);
    let (term_name, support, _detected_via) = term::identify_terminal();

    let _ = writeln!(o, "clipf {VERSION} - environment check\n");

    let _ = writeln!(o, "session");
    let _ = writeln!(o, "  os               {}", std::env::consts::OS);
    let _ = writeln!(o, "  arch             {}", std::env::consts::ARCH);
    let _ = writeln!(o, "  TERM             {}", envs("TERM"));
    let _ = writeln!(o, "  DISPLAY          {}", envs("DISPLAY"));
    let _ = writeln!(o, "  WAYLAND_DISPLAY  {}", envs("WAYLAND_DISPLAY"));
    let _ = writeln!(o, "  ssh session      {}", yn(term::in_ssh()));
    let _ = writeln!(o, "  wsl              {}", yn(term::is_wsl()));
    let _ = writeln!(
        o,
        "  multiplexer      {}",
        match mux {
            Multiplexer::Tmux => "tmux",
            Multiplexer::Screen => "screen",
            Multiplexer::None => "none",
        }
    );
    let _ = writeln!(o, "  local terminal   {term_name}");
    let _ = writeln!(
        o,
        "  escape output    {}{}",
        sink.name(),
        if sink.is_tty() {
            ""
        } else {
            "  (no controlling terminal)"
        }
    );

    let _ = writeln!(o, "\nhelpers");
    for t in [
        "xclip",
        "xsel",
        "wl-copy",
        "pbcopy",
        "clip.exe",
        "powershell.exe",
        "termux-clipboard-set",
        "wl-paste",
        "pbpaste",
        "termux-clipboard-get",
    ] {
        match which(t) {
            Some(p) => {
                let _ = writeln!(o, "  {t:<22} {}", p.display());
            }
            None => {
                let _ = writeln!(o, "  {t:<22} -");
            }
        }
    }

    let _ = writeln!(o, "\nbackend");
    let _ = writeln!(o, "  selected         {}", chosen.name());
    if cfg.backend.is_some() {
        let _ = writeln!(o, "  source           explicit (--backend / CLIPF_BACKEND)");
    } else {
        let _ = writeln!(o, "  source           auto-detected");
    }
    let _ = writeln!(
        o,
        "  size guard       {}",
        if cfg.max_bytes == 0 {
            "unlimited".to_string()
        } else {
            format!("{} bytes", cfg.max_bytes)
        }
    );

    let _ = writeln!(o, "\nnotes");
    let mut notes = 0;

    if let Some(cmd) = chosen.command() {
        if which(cmd).is_none() {
            let _ = writeln!(o, "  ! {cmd} is not installed, so this backend will fail.");
            notes += 1;
        }
    }

    // The classic remote-session trap.
    if term::in_ssh() && chosen != Backend::Osc52 {
        let _ = writeln!(
            o,
            "  ! This is an SSH session but a local clipboard tool was selected.\n\
             \x20   It would set the clipboard on the SERVER's display, not on your\n\
             \x20   machine. Use 'clipf -o FILE' to send the data to your terminal."
        );
        notes += 1;
    }

    if matches!(chosen, Backend::Xclip | Backend::Xsel) {
        let _ = writeln!(
            o,
            "  * Using the CLIPBOARD selection (Ctrl+V), not PRIMARY (middle-click)."
        );
        notes += 1;
    }

    if chosen == Backend::ClipExe {
        let _ = writeln!(
            o,
            "  * clip.exe mangles non-ASCII UTF-8, so text payloads are transcoded\n\
             \x20   to UTF-16LE automatically. Binary payloads are sent unchanged."
        );
        notes += 1;
    }

    if chosen == Backend::Osc52 {
        match support {
            Osc52Support::Yes => {
                let _ = writeln!(o, "  * {term_name} supports OSC 52. Should work as-is.");
            }
            Osc52Support::No => {
                let _ = writeln!(
                    o,
                    "  ! {term_name} does not support OSC 52 writes. Copying will\n\
                     \x20   silently do nothing. Use a different terminal, or pipe from\n\
                     \x20   your local shell:  ssh HOST 'cat FILE' | clipf"
                );
            }
            Osc52Support::Unknown => {
                let _ = writeln!(
                    o,
                    "  ? Could not identify the terminal, so OSC 52 support is unknown.\n\
                     \x20   Known good: kitty, iTerm2, WezTerm, Alacritty, foot, ghostty,\n\
                     \x20   Konsole, Windows Terminal, PuTTY 0.79+.\n\
                     \x20   Known bad: Apple Terminal, many older VTE builds."
                );
            }
        }
        notes += 1;

        if mux == Multiplexer::Tmux {
            let _ = writeln!(
                o,
                "  * tmux detected. Required in ~/.tmux.conf:\n\
                 \x20       set -g set-clipboard on\n\
                 \x20       set -g allow-passthrough on   # only needed for --tmux\n\
                 \x20   Existing panes keep the old setting, so run 'tmux kill-server'\n\
                 \x20   or start a fresh session after changing it."
            );
            notes += 1;
        }
        if mux == Multiplexer::Screen {
            let _ = writeln!(
                o,
                "  * screen detected. The payload will be split across several DCS\n\
                 \x20   passthrough chunks, which screen reassembles."
            );
            notes += 1;
        }

        let _ = writeln!(
            o,
            "  * OSC 52 payloads are size-capped and terminals truncate silently.\n\
             \x20   For large files, invert the direction and run locally:\n\
             \x20       ssh HOST 'cat FILE' | clipf"
        );
        notes += 1;

        if cfg!(windows) {
            let _ = writeln!(
                o,
                "  ! Native Windows consoles rarely act on OSC 52. Prefer the default\n\
                 \x20   backend here:  clipf --backend clip.exe FILE"
            );
            notes += 1;
        }
    }

    if notes == 0 {
        let _ = writeln!(o, "  nothing to flag - this environment looks fine.");
    }

    o
}

/// The `--check --json` form of `report`. Deliberately re-probes the
/// environment independently rather than sharing state with `report`, so
/// the human-readable report's existing, already-tested output can't be
/// perturbed by refactoring for this.
pub fn report_json(cfg: &Config) -> String {
    let sink = Sink::open();
    let mux = term::multiplexer();
    let chosen = cfg.backend.unwrap_or_else(Backend::detect);
    let (term_name, support, detected_via) = term::identify_terminal();

    let mut warnings: Vec<Value> = Vec::new();
    let mut warn = |code: &'static str, message: String| {
        warnings.push(Value::obj(vec![
            ("code", Value::str(code)),
            ("message", Value::str(message)),
        ]));
    };

    if let Some(cmd) = chosen.command() {
        if which(cmd).is_none() {
            warn(
                "backend_not_installed",
                format!("{cmd} is not installed, so this backend will fail."),
            );
        }
    }
    if term::in_ssh() && chosen != Backend::Osc52 {
        warn(
            "ssh_local_backend_selected",
            "This is an SSH session but a local clipboard tool was selected; it would \
             set the clipboard on the SERVER's display, not on your machine."
                .to_string(),
        );
    }
    if matches!(chosen, Backend::Xclip | Backend::Xsel) {
        warn(
            "xclip_xsel_clipboard_selection",
            "Using the CLIPBOARD selection (Ctrl+V), not PRIMARY (middle-click).".to_string(),
        );
    }
    if chosen == Backend::ClipExe {
        warn(
            "clip_exe_transcodes_utf16",
            "clip.exe mangles non-ASCII UTF-8, so text payloads are transcoded to \
             UTF-16LE automatically. Binary payloads are sent unchanged."
                .to_string(),
        );
    }
    if chosen == Backend::Osc52 {
        match support {
            Osc52Support::Yes => {
                warn(
                    "osc52_supported",
                    format!("{term_name} supports OSC 52. Should work as-is."),
                );
            }
            Osc52Support::No => {
                warn(
                    "osc52_unsupported",
                    format!(
                        "{term_name} does not support OSC 52 writes. Copying will \
                         silently do nothing."
                    ),
                );
            }
            Osc52Support::Unknown => {
                warn(
                    "osc52_support_unknown",
                    "Could not identify the terminal, so OSC 52 support is unknown."
                        .to_string(),
                );
            }
        }
        if mux == Multiplexer::Tmux {
            warn(
                "tmux_detected",
                "tmux detected; requires 'set -g set-clipboard on' in ~/.tmux.conf."
                    .to_string(),
            );
        }
        if mux == Multiplexer::Screen {
            warn(
                "screen_detected",
                "screen detected; the payload will be split across several DCS \
                 passthrough chunks, which screen reassembles."
                    .to_string(),
            );
        }
        warn(
            "osc52_size_capped",
            "OSC 52 payloads are size-capped and terminals truncate silently.".to_string(),
        );
        if cfg!(windows) {
            warn(
                "windows_console_osc52_unreliable",
                "Native Windows consoles rarely act on OSC 52.".to_string(),
            );
        }
    }

    let backend = Value::obj(vec![
        ("selected", Value::str(chosen.name())),
        ("available", Value::str_array(available_backend_names())),
        (
            "source",
            Value::str(match cfg.backend_source {
                BackendSource::Auto => "auto",
                BackendSource::Flag => "flag",
                BackendSource::Env => "env",
            }),
        ),
    ]);

    let multiplexer = match mux {
        Multiplexer::Tmux => Value::str("tmux"),
        Multiplexer::Screen => Value::str("screen"),
        Multiplexer::None => Value::Null,
    };

    let emulator = match detected_via {
        Some(v) => Value::obj(vec![
            ("name", Value::str(term_name)),
            ("detected_via", Value::str(v)),
        ]),
        None => Value::Null,
    };

    let osc52 = Value::str(match support {
        Osc52Support::Yes => "supported",
        Osc52Support::No => "unsupported",
        Osc52Support::Unknown => "unknown",
    });

    let value = Value::obj(vec![
        ("schema", Value::UInt(1)),
        ("clipf", Value::str(VERSION)),
        ("os", Value::str(std::env::consts::OS)),
        ("backend", backend),
        ("multiplexer", multiplexer),
        ("emulator", emulator),
        ("osc52", osc52),
        ("tty", Value::str(sink.name())),
        ("ssh", Value::Bool(term::in_ssh())),
        ("max_bytes", Value::UInt(cfg.max_bytes as u64)),
        ("warnings", Value::Array(warnings)),
    ]);

    json::write(&value)
}

/// Every backend that has a local helper on `PATH` right now, plus `osc52`
/// which is always available as the fallback.
fn available_backend_names() -> Vec<&'static str> {
    let mut avail: Vec<&'static str> = [
        Backend::PbCopy,
        Backend::ClipExe,
        Backend::Termux,
        Backend::WlCopy,
        Backend::Xclip,
        Backend::Xsel,
    ]
    .into_iter()
    .filter(|b| matches!(b.command(), Some(cmd) if which(cmd).is_some()))
    .map(Backend::name)
    .collect();
    avail.push(Backend::Osc52.name());
    avail
}

fn envs(k: &str) -> String {
    env::var(k).unwrap_or_else(|_| "<unset>".into())
}

fn yn(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_covers_the_main_sections() {
        let cfg = Config::default();
        let r = report(&cfg);
        for section in ["session", "helpers", "backend", "notes"] {
            assert!(r.contains(section), "missing section: {section}");
        }
        assert!(r.contains("clipf "));
    }

    #[test]
    fn report_shows_an_explicit_backend_as_explicit() {
        let cfg = Config {
            backend: Some(Backend::Xsel),
            ..Config::default()
        };
        let r = report(&cfg);
        assert!(r.contains("xsel"));
        assert!(r.contains("explicit"));
    }

    #[test]
    fn unlimited_guard_is_labelled() {
        let cfg = Config {
            max_bytes: 0,
            ..Config::default()
        };
        assert!(report(&cfg).contains("unlimited"));
    }

    #[test]
    fn report_names_the_host_architecture() {
        assert!(report(&Config::default()).contains(std::env::consts::ARCH));
    }

    #[test]
    fn report_json_has_the_documented_top_level_shape() {
        // Fully deterministic across CI machines only for structure, not
        // for values that depend on installed helpers or the real
        // terminal (those are exercised end to end in the parrot smoke
        // test, not here).
        let cfg = Config::default();
        let out = report_json(&cfg);
        assert!(out.starts_with('{') && out.ends_with('}'), "not an object: {out}");
        assert!(!out.contains(",}"), "trailing comma before }} in {out}");
        assert!(!out.contains(",]"), "trailing comma before ] in {out}");
        for key in [
            "schema", "clipf", "os", "backend", "multiplexer", "emulator", "osc52", "tty",
            "ssh", "max_bytes", "warnings",
        ] {
            assert!(
                out.contains(&format!("\"{key}\":")),
                "missing top-level key {key} in {out}"
            );
        }
        assert!(out.contains("\"schema\":1"));
        assert!(out.contains(&format!("\"clipf\":\"{VERSION}\"")));
        assert!(out.contains(&format!("\"os\":\"{}\"", std::env::consts::OS)));
    }

    #[test]
    fn report_json_backend_source_reflects_config() {
        let auto = report_json(&Config::default());
        assert!(auto.contains("\"source\":\"auto\""));

        let flagged = Config {
            backend: Some(Backend::Osc52),
            backend_source: BackendSource::Flag,
            ..Config::default()
        };
        assert!(report_json(&flagged).contains("\"source\":\"flag\""));
        assert!(report_json(&flagged).contains("\"selected\":\"osc52\""));
    }

    #[test]
    fn report_json_max_bytes_reflects_config() {
        let cfg = Config {
            max_bytes: 12345,
            ..Config::default()
        };
        assert!(report_json(&cfg).contains("\"max_bytes\":12345"));
    }

    #[test]
    fn available_backends_always_include_osc52() {
        assert!(available_backend_names().contains(&"osc52"));
    }
}
