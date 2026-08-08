//! Argument parsing. Hand-rolled to keep the dependency count at zero.

use std::path::PathBuf;

use crate::backend::Backend;
use crate::completions::Shell;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_MAX: usize = 65536;

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Copy,
    Paste,
    Check,
    Update,
    Help,
    Version,
    Completions(Shell),
}

/// How `Config::backend` ended up set — reported verbatim as
/// `backend.source` in `--check --json` output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSource {
    Auto,
    Flag,
    Env,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub action: Action,
    pub file: Option<PathBuf>,
    pub omp_session: Option<String>,
    pub omp_raw: Option<String>,
    pub backend: Option<Backend>,
    pub backend_source: BackendSource,
    pub strip_newline: bool,
    pub tee: bool,
    pub passthrough: bool,
    pub max_bytes: usize,
    pub force: bool,
    pub verbose: bool,
    pub dry_run: bool,
    pub json: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            action: Action::Copy,
            file: None,
            omp_session: None,
            omp_raw: None,
            backend: None,
            backend_source: BackendSource::Auto,
            strip_newline: false,
            tee: false,
            passthrough: false,
            max_bytes: DEFAULT_MAX,
            force: false,
            verbose: false,
            dry_run: false,
            json: false,
        }
    }
}

impl Config {
    /// Environment defaults, applied before the command line so flags win.
    pub fn from_env() -> Self {
        let mut c = Config::default();
        if let Ok(v) = std::env::var("CLIPF_BACKEND") {
            if !v.is_empty() {
                if let Some(b) = Backend::parse(&v) {
                    c.backend = Some(b);
                    c.backend_source = BackendSource::Env;
                }
            }
        }
        if let Ok(v) = std::env::var("CLIPF_MAX_BYTES") {
            if let Ok(n) = v.parse() {
                c.max_bytes = n;
            }
        }
        if std::env::var("CLIPF_TMUX_PASSTHROUGH").map(|v| v == "1").unwrap_or(false) {
            c.passthrough = true;
        }
        c
    }
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Config, String> {
    let mut cfg = Config::from_env();
    let mut it = args.into_iter().peekable();
    let mut positional_seen = false;
    let mut no_more_flags = false;

    while let Some(arg) = it.next() {
        if no_more_flags || !arg.starts_with('-') || arg == "-" {
            if arg == "update" || arg == "upgrade" {
                cfg.action = Action::Update;
                continue;
            }
            if positional_seen {
                return Err("more than one FILE given".into());
            }
            positional_seen = true;
            if arg != "-" {
                cfg.file = Some(PathBuf::from(arg));
            }
            continue;
        }

        // Split --opt=value.
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };

        let mut take_value = |what: &str| -> Result<String, String> {
            if let Some(v) = inline.clone() {
                Ok(v)
            } else {
                it.next()
                    .ok_or_else(|| format!("{what} requires an argument"))
            }
        };

        match name.as_str() {
            "--" => no_more_flags = true,
            "-h" | "--help" => return Ok(Config { action: Action::Help, ..cfg }),
            "-V" | "--version" => return Ok(Config { action: Action::Version, ..cfg }),
            "--check" => cfg.action = Action::Check,
            "-O" | "--paste" => cfg.action = Action::Paste,
            "-n" | "--no-newline" => cfg.strip_newline = true,
            "-p" | "--print" => cfg.tee = true,
            "-t" | "--tmux" => cfg.passthrough = true,
            "--update" | "--upgrade" => cfg.action = Action::Update,
            "-f" | "--force" => cfg.force = true,
            "-v" | "--verbose" => cfg.verbose = true,
            "--dry-run" => cfg.dry_run = true,
            "--json" => cfg.json = true,
            "--omp" | "--omp-session" => {
                if let Some(v) = inline.clone() {
                    cfg.omp_session = Some(if v.is_empty() { "latest".to_string() } else { v });
                } else if let Some(next_arg) = it.peek() {
                    if !next_arg.starts_with('-') {
                        cfg.omp_session = Some(it.next().unwrap());
                    } else {
                        cfg.omp_session = Some("latest".to_string());
                    }
                } else {
                    cfg.omp_session = Some("latest".to_string());
                }
            }
            "--omp-raw" | "--omp-jsonl" => {
                if let Some(v) = inline.clone() {
                    cfg.omp_raw = Some(if v.is_empty() { "latest".to_string() } else { v });
                } else if let Some(next_arg) = it.peek() {
                    if !next_arg.starts_with('-') {
                        cfg.omp_raw = Some(it.next().unwrap());
                    } else {
                        cfg.omp_raw = Some("latest".to_string());
                    }
                } else {
                    cfg.omp_raw = Some("latest".to_string());
                }
            }
            "--completions" => {
                let v = take_value("--completions")?;
                let shell = Shell::parse(&v)
                    .ok_or_else(|| format!("unknown shell: {v} (expected bash, zsh, or fish)"))?;
                return Ok(Config {
                    action: Action::Completions(shell),
                    ..cfg
                });
            }
            "-o" | "--osc52" => {
                cfg.backend = Some(Backend::Osc52);
                cfg.backend_source = BackendSource::Flag;
            }
            "-b" | "--backend" => {
                let v = take_value("--backend")?;
                cfg.backend = Some(
                    Backend::parse(&v)
                        .ok_or_else(|| format!("unknown backend: {v} (see --help)"))?,
                );
                cfg.backend_source = BackendSource::Flag;
            }
            "-m" | "--max" => {
                let v = take_value("--max")?;
                cfg.max_bytes = v
                    .parse()
                    .map_err(|_| format!("--max must be a number, got: {v}"))?;
            }
            // Clustered short flags such as -nv or -pvo.
            s if s.len() > 2 && !s.starts_with("--") => {
                for ch in s[1..].chars() {
                    match ch {
                        'n' => cfg.strip_newline = true,
                        'p' => cfg.tee = true,
                        't' => cfg.passthrough = true,
                        'f' => cfg.force = true,
                        'v' => cfg.verbose = true,
                        'o' => {
                            cfg.backend = Some(Backend::Osc52);
                            cfg.backend_source = BackendSource::Flag;
                        }
                        'O' => cfg.action = Action::Paste,
                        _ => {
                            return Err(format!(
                                "unknown option: -{ch} in {s} (flags taking a value \
                                 cannot be clustered; try --help)"
                            ))
                        }
                    }
                }
            }
            other => return Err(format!("unknown option: {other} (try --help)")),
        }
    }

    // --json owns stdout for its structured object; -p/--print and
    // -O/--paste already write the payload itself there. Reject the
    // combination here rather than at emit time, so it's a clean usage
    // error instead of a mangled stream.
    if cfg.json && (cfg.tee || cfg.action == Action::Paste) {
        return Err(
            "--json cannot be combined with -p/--print or -O/--paste (both already write to stdout)"
                .into(),
        );
    }

    Ok(cfg)
}

pub fn help() -> String {
    format!(
        "\
clipf {VERSION} - copy file contents to the clipboard

USAGE
    clipf [OPTIONS] [FILE]
    <command> | clipf [OPTIONS]

    Reads FILE, or stdin when FILE is omitted or \"-\".

OPTIONS
    -n, --no-newline     strip trailing newline(s) (good for tokens, IPs, hashes)
    -p, --print          also write the content to stdout (tee behaviour)
    -b, --backend NAME   force backend: auto|osc52|xclip|xsel|wl|pbcopy|clip.exe
    -o, --osc52          shorthand for --backend osc52
    -t, --tmux           wrap OSC 52 in a tmux DCS passthrough
    -m, --max BYTES      OSC 52 size guard (default {DEFAULT_MAX}, 0 = unlimited)
    -f, --force          copy even if it exceeds the size guard
    -O, --paste          print the current clipboard instead of copying
        --check          diagnose this environment and exit
        --dry-run        show what would happen, copy nothing
        --json           machine-readable output on stdout (copy or --check;
                          not combinable with -p/--print or -O/--paste)
        --completions SHELL   print a completion script: bash|zsh|fish
        --omp-session [ID], --omp [ID]  copy OMP session transcript (default: latest)
    -v, --verbose        report the chosen backend and byte count
        --omp-raw [ID], --omp-jsonl [ID]  copy raw OMP session .jsonl file as pasteable shell script
        --update, update       update clipf to the latest release
    -h, --help           this text
    -V, --version        version

ENVIRONMENT
    CLIPF_BACKEND            same as --backend
    CLIPF_MAX_BYTES          same as --max
    CLIPF_TMUX_PASSTHROUGH   set to 1 to default --tmux on
    CLIPF_NO_SECRET_WARN     set to 1 to silence the private-key/API-token warning

EXAMPLES
    clipf server.conf                     # copy a config file
    clipf -n token.txt                    # copy without the trailing newline
    grep -v '^#' fw-rules.sh | clipf      # copy filtered output
    ssh ovpn1 'cat /etc/x.conf' | clipf   # run locally, no size limit
    clipf --check                         # \"why isn't this working?\"
    clipf --check --json                  # same, for scripts/agents
    clipf --completions zsh >> ~/.zshrc   # (or your shell's completion dir)

EXIT CODES  (changed in 0.5.0 - see README for the full table)
    0  copied (or --dry-run/--check/--help/--version)
    1  usage error
    3  refused: exceeds the OSC 52 size guard
    4  input error (missing file, directory, permission denied)
    5  backend unavailable (helper not installed)
    6  backend failed (helper exited non-zero, or a write failed)
    8  --paste against a backend that can't be read back (OSC 52)

NOTES
    Over SSH, install nothing on the server: OSC 52 hands the data to your local
    terminal emulator. Inside tmux this needs 'set -g set-clipboard on', and the
    payload is size-limited - for large files prefer piping an ssh command into
    clipf on your local machine.
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Config, String> {
        // Isolate from the ambient environment so tests are deterministic.
        std::env::remove_var("CLIPF_BACKEND");
        std::env::remove_var("CLIPF_MAX_BYTES");
        std::env::remove_var("CLIPF_TMUX_PASSTHROUGH");
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults() {
        let c = p(&[]).unwrap();
        assert_eq!(c.action, Action::Copy);
        assert_eq!(c.file, None);
        assert_eq!(c.backend, None);
        assert_eq!(c.backend_source, BackendSource::Auto);
        assert_eq!(c.max_bytes, DEFAULT_MAX);
        assert!(!c.strip_newline && !c.tee && !c.force && !c.verbose && !c.dry_run && !c.json);
    }

    #[test]
    fn positional_file() {
        assert_eq!(p(&["a.txt"]).unwrap().file, Some(PathBuf::from("a.txt")));
    }

    #[test]
    fn dash_means_stdin() {
        assert_eq!(p(&["-"]).unwrap().file, None);
    }

    #[test]
    fn rejects_two_files() {
        assert!(p(&["a", "b"]).is_err());
    }

    #[test]
    fn long_and_short_flags_agree() {
        assert!(p(&["-n"]).unwrap().strip_newline);
        assert!(p(&["--no-newline"]).unwrap().strip_newline);
        assert!(p(&["-p"]).unwrap().tee);
        assert!(p(&["--print"]).unwrap().tee);
    }

    #[test]
    fn backend_flag_forms() {
        assert_eq!(p(&["-b", "xclip"]).unwrap().backend, Some(Backend::Xclip));
        assert_eq!(p(&["--backend", "wl"]).unwrap().backend, Some(Backend::WlCopy));
        assert_eq!(p(&["--backend=pbcopy"]).unwrap().backend, Some(Backend::PbCopy));
        assert_eq!(p(&["-o"]).unwrap().backend, Some(Backend::Osc52));
        assert!(p(&["-b", "junk"]).is_err());
        assert!(p(&["-b"]).is_err());
    }

    #[test]
    fn explicit_backend_flag_is_recorded_as_flag_source() {
        assert_eq!(p(&["-b", "xclip"]).unwrap().backend_source, BackendSource::Flag);
        assert_eq!(p(&["-o"]).unwrap().backend_source, BackendSource::Flag);
        assert_eq!(p(&[]).unwrap().backend_source, BackendSource::Auto);
    }

    #[test]
    fn max_parsing() {
        assert_eq!(p(&["-m", "100"]).unwrap().max_bytes, 100);
        assert_eq!(p(&["--max=0"]).unwrap().max_bytes, 0);
        assert!(p(&["--max", "abc"]).is_err());
        assert!(p(&["--max"]).is_err());
    }

    #[test]
    fn clustered_short_flags() {
        let c = p(&["-nvp"]).unwrap();
        assert!(c.strip_newline && c.verbose && c.tee);
        // A value-taking flag inside a cluster is a hard error rather than a
        // silent misparse.
        assert!(p(&["-nb"]).is_err());
    }

    #[test]
    fn double_dash_stops_flag_parsing() {
        let c = p(&["--", "-weird-name.txt"]).unwrap();
        assert_eq!(c.file, Some(PathBuf::from("-weird-name.txt")));
    }

    #[test]
    fn help_and_version_short_circuit() {
        assert_eq!(p(&["--help", "--nonsense"]).unwrap().action, Action::Help);
        assert_eq!(p(&["-V"]).unwrap().action, Action::Version);
    }

    #[test]
    fn unknown_option_is_rejected() {
        assert!(p(&["--nope"]).is_err());
    }

    #[test]
    fn json_flag_is_recognised_and_defaults_off() {
        assert!(!p(&[]).unwrap().json);
        assert!(p(&["--json"]).unwrap().json);
        assert!(p(&["--check", "--json"]).unwrap().json);
    }

    #[test]
    fn json_conflicts_with_print_and_paste() {
        assert!(p(&["--json", "-p"]).is_err());
        assert!(p(&["-p", "--json"]).is_err());
        assert!(p(&["--json", "-O"]).is_err());
        assert!(p(&["-O", "--json"]).is_err());
        // --check --json is explicitly fine.
        assert!(p(&["--check", "--json"]).is_ok());
        // plain copy with --json is fine.
        assert!(p(&["--json", "file.txt"]).is_ok());
    }

    #[test]
    fn completions_flag_selects_the_shell() {
        assert_eq!(
            p(&["--completions", "bash"]).unwrap().action,
            Action::Completions(Shell::Bash)
        );
        assert_eq!(
            p(&["--completions", "zsh"]).unwrap().action,
            Action::Completions(Shell::Zsh)
        );
        assert_eq!(
            p(&["--completions", "fish"]).unwrap().action,
            Action::Completions(Shell::Fish)
        );
        assert_eq!(
            p(&["--completions=bash"]).unwrap().action,
            Action::Completions(Shell::Bash)
        );
        assert!(p(&["--completions", "powershell"]).is_err());
        assert!(p(&["--completions"]).is_err());
    }
    #[test]
    fn omp_session_flag_parsing() {
        assert_eq!(
            p(&["--omp"]).unwrap().omp_session,
            Some("latest".to_string())
        );
        assert_eq!(
            p(&["--omp-session"]).unwrap().omp_session,
            Some("latest".to_string())
        );
        assert_eq!(
            p(&["--omp", "019fc02a"]).unwrap().omp_session,
            Some("019fc02a".to_string())
        );
        assert_eq!(
            p(&["--omp=019fc02a"]).unwrap().omp_session,
            Some("019fc02a".to_string())
        );
        assert_eq!(
            p(&["--omp-session", "latest"]).unwrap().omp_session,
            Some("latest".to_string())
        );
        // Followed by another flag
        let c = p(&["--omp", "--dry-run"]).unwrap();
        assert_eq!(c.omp_session, Some("latest".to_string()));
        assert!(c.dry_run);

        assert_eq!(
            p(&["--omp-raw"]).unwrap().omp_raw,
            Some("latest".to_string())
        );
        assert_eq!(
            p(&["--omp-jsonl"]).unwrap().omp_raw,
            Some("latest".to_string())
        );
        assert_eq!(
            p(&["--omp-raw", "019fc02a"]).unwrap().omp_raw,
            Some("019fc02a".to_string())
        );
    }
    #[test]
    fn update_flag_parsing() {
        assert_eq!(p(&["update"]).unwrap().action, Action::Update);
        assert_eq!(p(&["upgrade"]).unwrap().action, Action::Update);
        assert_eq!(p(&["--update"]).unwrap().action, Action::Update);
        assert_eq!(p(&["--upgrade"]).unwrap().action, Action::Update);
    }
    #[test]
    fn help_text_mentions_every_flag() {
        let h = help();
        for flag in [
            "--no-newline", "--print", "--backend", "--osc52", "--tmux", "--max",
            "--force", "--paste", "--check", "--dry-run", "--verbose", "--json",
            "--completions", "--omp-session", "--omp", "--omp-raw", "--omp-jsonl", "--update", "--help", "--version",
        ] {
            assert!(h.contains(flag), "help is missing {flag}");
        }
    }

    #[test]
    fn help_text_exit_codes_match_the_exit_module() {
        use crate::exit::ErrorKind;
        let h = help();
        for kind in [
            ErrorKind::Usage,
            ErrorKind::Input,
            ErrorKind::BackendUnavailable,
            ErrorKind::BackendFailed,
            ErrorKind::PasteUnsupported,
        ] {
            // Matches this file's own "    N  description" formatting exactly.
            let marker = format!("\n    {}  ", kind.code());
            assert!(
                h.contains(&marker),
                "help text's EXIT CODES section is missing code {}",
                kind.code()
            );
        }
    }
}
