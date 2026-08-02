//! Clipboard backends: the local helper tools, plus OSC 52 as the fallback
//! that works over SSH.

use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::term;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Osc52,
    Xclip,
    Xsel,
    WlCopy,
    PbCopy,
    ClipExe,
}

impl Backend {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::detect()),
            "osc52" | "osc" => Some(Backend::Osc52),
            "xclip" => Some(Backend::Xclip),
            "xsel" => Some(Backend::Xsel),
            "wl" | "wl-copy" | "wayland" => Some(Backend::WlCopy),
            "pbcopy" | "macos" => Some(Backend::PbCopy),
            "clip.exe" | "clip" | "wsl" => Some(Backend::ClipExe),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Backend::Osc52 => "osc52",
            Backend::Xclip => "xclip",
            Backend::Xsel => "xsel",
            Backend::WlCopy => "wl",
            Backend::PbCopy => "pbcopy",
            Backend::ClipExe => "clip.exe",
        }
    }

    /// The external command this backend shells out to, if any.
    pub fn command(self) -> Option<&'static str> {
        match self {
            Backend::Osc52 => None,
            Backend::Xclip => Some("xclip"),
            Backend::Xsel => Some("xsel"),
            Backend::WlCopy => Some("wl-copy"),
            Backend::PbCopy => Some("pbcopy"),
            Backend::ClipExe => Some("clip.exe"),
        }
    }

    /// Pick a backend for this environment. OSC 52 is the last resort because
    /// it is the only one with a size limit, but on a headless server it is
    /// also the only one that reaches the user's actual clipboard.
    pub fn detect() -> Self {
        if which("pbcopy").is_some() {
            Backend::PbCopy
        } else if term::is_wsl() && which("clip.exe").is_some() {
            Backend::ClipExe
        } else if env::var_os("WAYLAND_DISPLAY").is_some() && which("wl-copy").is_some() {
            Backend::WlCopy
        } else if env::var_os("DISPLAY").is_some() && which("xclip").is_some() {
            Backend::Xclip
        } else if env::var_os("DISPLAY").is_some() && which("xsel").is_some() {
            Backend::Xsel
        } else {
            Backend::Osc52
        }
    }

    fn copy_args(self) -> &'static [&'static str] {
        match self {
            // Default to CLIPBOARD, not PRIMARY. Forgetting this is the single
            // most common reason "xclip didn't work" - PRIMARY only pastes via
            // middle-click, not Ctrl+V.
            Backend::Xclip => &["-selection", "clipboard"],
            Backend::Xsel => &["--input", "--clipboard"],
            _ => &[],
        }
    }

    fn paste_command(self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Backend::Xclip => Some(("xclip", &["-selection", "clipboard", "-o"])),
            Backend::Xsel => Some(("xsel", &["--output", "--clipboard"])),
            Backend::WlCopy => Some(("wl-paste", &["-n"])),
            Backend::PbCopy => Some(("pbpaste", &[])),
            Backend::ClipExe => Some((
                "powershell.exe",
                &["-NoProfile", "-Command", "Get-Clipboard"],
            )),
            Backend::Osc52 => None,
        }
    }

    /// Pipe `data` into the local helper tool.
    pub fn copy_local(self, data: &[u8]) -> Result<(), String> {
        let cmd = self
            .command()
            .ok_or_else(|| "osc52 is not an external command".to_string())?;

        let mut child = Command::new(cmd)
            .args(self.copy_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("cannot run {cmd}: {e}"))?;

        // Take stdin and drop it so the child sees EOF; xclip and wl-copy both
        // wait for the stream to close before daemonising.
        {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| format!("{cmd}: no stdin pipe"))?;
            stdin
                .write_all(data)
                .map_err(|e| format!("writing to {cmd}: {e}"))?;
        }

        let status = child
            .wait()
            .map_err(|e| format!("waiting for {cmd}: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{cmd} exited with {status}"))
        }
    }

    pub fn paste(self) -> Result<Vec<u8>, String> {
        let (cmd, args) = self.paste_command().ok_or_else(|| {
            "reading the clipboard over OSC 52 is not supported by terminals \
             (the query form is disabled almost everywhere as a security measure)"
                .to_string()
        })?;

        let out = Command::new(cmd)
            .args(args)
            .output()
            .map_err(|e| format!("cannot run {cmd}: {e}"))?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(format!("{cmd} exited with {}", out.status))
        }
    }
}

/// `which(1)` without spawning anything.
pub fn which(cmd: &str) -> Option<PathBuf> {
    if cmd.contains('/') {
        let p = PathBuf::from(cmd);
        return is_executable(&p).then_some(p);
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(cmd))
        .find(|p| is_executable(p))
}

fn is_executable(p: &std::path::Path) -> bool {
    // Checking the mode bits is enough here: this only decides which helper to
    // try, and a genuine exec failure is reported by copy_local anyway.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_names_and_aliases() {
        assert_eq!(Backend::parse("osc52"), Some(Backend::Osc52));
        assert_eq!(Backend::parse("OSC52"), Some(Backend::Osc52));
        assert_eq!(Backend::parse("wl-copy"), Some(Backend::WlCopy));
        assert_eq!(Backend::parse("wayland"), Some(Backend::WlCopy));
        assert_eq!(Backend::parse("clip"), Some(Backend::ClipExe));
        assert_eq!(Backend::parse("nonsense"), None);
    }

    #[test]
    fn names_round_trip_through_parse() {
        for b in [
            Backend::Osc52,
            Backend::Xclip,
            Backend::Xsel,
            Backend::WlCopy,
            Backend::PbCopy,
            Backend::ClipExe,
        ] {
            assert_eq!(Backend::parse(b.name()), Some(b), "{}", b.name());
        }
    }

    #[test]
    fn x11_backends_target_the_clipboard_selection() {
        assert!(Backend::Xclip.copy_args().contains(&"clipboard"));
        assert!(Backend::Xsel.copy_args().contains(&"--clipboard"));
    }

    #[test]
    fn osc52_has_no_external_command() {
        assert!(Backend::Osc52.command().is_none());
        assert!(Backend::Osc52.copy_local(b"x").is_err());
        assert!(Backend::Osc52.paste().is_err());
    }

    #[test]
    fn which_finds_a_standard_binary_and_rejects_nonsense() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-real-binary-xyz").is_none());
    }
}
