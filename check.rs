//! `--check`: answer "why isn't this working?" without a round of guessing.

use std::env;
use std::fmt::Write as _;

use crate::backend::{which, Backend};
use crate::cli::{Config, VERSION};
use crate::term::{self, Multiplexer, Osc52Support, Sink};

pub fn report(cfg: &Config) -> String {
    let mut o = String::new();
    let sink = Sink::open();
    let mux = term::multiplexer();
    let chosen = cfg.backend.unwrap_or_else(Backend::detect);
    let (term_name, support) = term::identify_terminal();

    let _ = writeln!(o, "clipf {VERSION} - environment check\n");

    let _ = writeln!(o, "session");
    let _ = writeln!(o, "  os               {}", std::env::consts::OS);
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
    for t in ["xclip", "xsel", "wl-copy", "pbcopy", "clip.exe", "wl-paste", "pbpaste"] {
        match which(t) {
            Some(p) => {
                let _ = writeln!(o, "  {t:<10} {}", p.display());
            }
            None => {
                let _ = writeln!(o, "  {t:<10} -");
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
    }

    if notes == 0 {
        let _ = writeln!(o, "  nothing to flag - this environment looks fine.");
    }

    o
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
}
