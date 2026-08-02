//! Terminal and session probing.

use std::env;
use std::fs::OpenOptions;
use std::io::{self, Write};

/// Where raw escape sequences go. Never stdout, which may be redirected into
/// a file or another process.
pub enum Sink {
    Tty(std::fs::File),
    Stderr,
}

impl Sink {
    /// Open the controlling terminal, falling back to stderr.
    ///
    /// Checking the mode bits with `access(2)` is not enough: `/dev/tty` can be
    /// writable by permission while `open(2)` still fails with ENXIO because the
    /// process has no controlling terminal (a daemon, a `setsid` child, a CI
    /// runner). The only reliable probe is to actually open it.
    pub fn open() -> Self {
        match OpenOptions::new().write(true).open("/dev/tty") {
            Ok(f) => Sink::Tty(f),
            Err(_) => Sink::Stderr,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Sink::Tty(_) => "/dev/tty",
            Sink::Stderr => "stderr",
        }
    }

    pub fn is_tty(&self) -> bool {
        matches!(self, Sink::Tty(_))
    }

    pub fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        match self {
            Sink::Tty(f) => {
                f.write_all(buf)?;
                f.flush()
            }
            Sink::Stderr => {
                let mut e = io::stderr().lock();
                e.write_all(buf)?;
                e.flush()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplexer {
    None,
    Tmux,
    Screen,
}

pub fn multiplexer() -> Multiplexer {
    if env::var_os("TMUX").is_some() {
        Multiplexer::Tmux
    } else if env::var_os("STY").is_some()
        || env::var("TERM").map(|t| t.starts_with("screen")).unwrap_or(false)
    {
        // A bare `screen*` TERM without STY is usually tmux with a legacy
        // TERM setting, but TMUX was already checked above, so treat it as
        // screen and use the conservative chunked path.
        Multiplexer::Screen
    } else {
        Multiplexer::None
    }
}

pub fn in_ssh() -> bool {
    env::var_os("SSH_CONNECTION").is_some() || env::var_os("SSH_TTY").is_some()
}

pub fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|v| v.to_ascii_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

/// Best guess at the local terminal emulator, and whether it speaks OSC 52.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Osc52Support {
    Yes,
    No,
    Unknown,
}

pub fn identify_terminal() -> (String, Osc52Support) {
    // Emulator-specific variables are far more reliable than TERM, which is
    // usually just "xterm-256color" no matter what is really running.
    if env::var_os("KITTY_WINDOW_ID").is_some() {
        return ("kitty".into(), Osc52Support::Yes);
    }
    if env::var_os("ALACRITTY_SOCKET").is_some() || env::var_os("ALACRITTY_WINDOW_ID").is_some() {
        return ("alacritty".into(), Osc52Support::Yes);
    }
    if env::var_os("WEZTERM_PANE").is_some() || env::var_os("WEZTERM_EXECUTABLE").is_some() {
        return ("wezterm".into(), Osc52Support::Yes);
    }
    if env::var_os("WT_SESSION").is_some() {
        return ("windows terminal".into(), Osc52Support::Yes);
    }
    if env::var_os("KONSOLE_VERSION").is_some() {
        return ("konsole".into(), Osc52Support::Yes);
    }
    if let Ok(tp) = env::var("TERM_PROGRAM") {
        let s = match tp.as_str() {
            "iTerm.app" => Osc52Support::Yes,
            "WezTerm" => Osc52Support::Yes,
            "ghostty" => Osc52Support::Yes,
            "vscode" => Osc52Support::Yes,
            "Apple_Terminal" => Osc52Support::No,
            _ => Osc52Support::Unknown,
        };
        return (tp.to_ascii_lowercase(), s);
    }
    if env::var_os("VTE_VERSION").is_some() {
        // GNOME Terminal and friends. VTE only gained OSC 52 write support
        // recently and many distro builds still lack it.
        return ("vte-based (gnome-terminal?)".into(), Osc52Support::Unknown);
    }

    let term = env::var("TERM").unwrap_or_else(|_| "<unset>".into());
    (format!("unknown (TERM={term})"), Osc52Support::Unknown)
}
