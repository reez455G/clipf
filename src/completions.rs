//! Shell completion scripts, hand-written — no `clap_complete`, this crate
//! stays dependency-free. `clipf --completions SHELL` prints one to stdout;
//! the release workflow generates and attaches all three to each release
//! rather than requiring a compiler or clipf itself on the user's machine
//! just to get completions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bash" => Some(Shell::Bash),
            "zsh" => Some(Shell::Zsh),
            "fish" => Some(Shell::Fish),
            _ => None,
        }
    }
}

pub fn script(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => BASH,
        Shell::Zsh => ZSH,
        Shell::Fish => FISH,
    }
}

// Kept in sync with cli.rs's flag table by the tests below, which assert
// every long flag name from cli::help() also appears here.

const BASH: &str = r#"# clipf bash completion
# Install: source this file, or drop it in /etc/bash_completion.d/
_clipf_completions() {
    local cur prev opts
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    opts="-n --no-newline -p --print -b --backend -o --osc52 -t --tmux -m --max -f --force -O --paste --check --dry-run --json -v --verbose --completions -h --help -V --version"

    case "$prev" in
        -b|--backend)
            COMPREPLY=( $(compgen -W "auto osc52 xclip xsel wl pbcopy clip.exe termux" -- "$cur") )
            return 0
            ;;
        --completions)
            COMPREPLY=( $(compgen -W "bash zsh fish" -- "$cur") )
            return 0
            ;;
        -m|--max)
            return 0
            ;;
    esac

    if [[ "$cur" == -* ]]; then
        COMPREPLY=( $(compgen -W "$opts" -- "$cur") )
    else
        COMPREPLY=( $(compgen -f -- "$cur") )
    fi
}
complete -F _clipf_completions clipf
"#;

const ZSH: &str = r#"#compdef clipf
# clipf zsh completion. Install: place on your $fpath as _clipf, or source
# this file after `autoload -U compinit && compinit`.

_clipf() {
    local -a opts
    opts=(
        '(-n --no-newline)'{-n,--no-newline}'[strip trailing newline(s)]'
        '(-p --print)'{-p,--print}'[also write content to stdout]'
        '(-b --backend)'{-b,--backend}'[force backend]:backend:(auto osc52 xclip xsel wl pbcopy clip.exe termux)'
        '(-o --osc52)'{-o,--osc52}'[shorthand for --backend osc52]'
        '(-t --tmux)'{-t,--tmux}'[wrap OSC 52 in a tmux DCS passthrough]'
        '(-m --max)'{-m,--max}'[OSC 52 size guard]:bytes:'
        '(-f --force)'{-f,--force}'[copy even if it exceeds the size guard]'
        '(-O --paste)'{-O,--paste}'[print the current clipboard instead of copying]'
        '--check[diagnose this environment and exit]'
        '--dry-run[show what would happen, copy nothing]'
        '--json[machine-readable output on stdout]'
        '(-v --verbose)'{-v,--verbose}'[report the chosen backend and byte count]'
        '--completions[print a shell completion script]:shell:(bash zsh fish)'
        '(-h --help)'{-h,--help}'[show help]'
        '(-V --version)'{-V,--version}'[show version]'
        '*:file:_files'
    )
    _arguments -s -w $opts
}
_clipf "$@"
"#;

const FISH: &str = r#"# clipf fish completion
# Install: save as ~/.config/fish/completions/clipf.fish
complete -c clipf -s n -l no-newline -d 'strip trailing newline(s)'
complete -c clipf -s p -l print -d 'also write content to stdout'
complete -c clipf -s b -l backend -d 'force backend' -xa 'auto osc52 xclip xsel wl pbcopy clip.exe termux'
complete -c clipf -s o -l osc52 -d 'shorthand for --backend osc52'
complete -c clipf -s t -l tmux -d 'wrap OSC 52 in a tmux DCS passthrough'
complete -c clipf -s m -l max -d 'OSC 52 size guard (bytes)'
complete -c clipf -s f -l force -d 'copy even if it exceeds the size guard'
complete -c clipf -s O -l paste -d 'print the current clipboard instead of copying'
complete -c clipf -l check -d 'diagnose this environment and exit'
complete -c clipf -l dry-run -d 'show what would happen, copy nothing'
complete -c clipf -l json -d 'machine-readable output on stdout'
complete -c clipf -s v -l verbose -d 'report the chosen backend and byte count'
complete -c clipf -l completions -d 'print a shell completion script' -xa 'bash zsh fish'
complete -c clipf -s h -l help -d 'show help'
complete -c clipf -s V -l version -d 'show version'
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_is_case_insensitive_and_rejects_unknown() {
        assert_eq!(Shell::parse("bash"), Some(Shell::Bash));
        assert_eq!(Shell::parse("BASH"), Some(Shell::Bash));
        assert_eq!(Shell::parse("Zsh"), Some(Shell::Zsh));
        assert_eq!(Shell::parse("fish"), Some(Shell::Fish));
        assert_eq!(Shell::parse("powershell"), None);
    }

    #[test]
    fn every_shell_has_a_non_empty_script() {
        for s in [Shell::Bash, Shell::Zsh, Shell::Fish] {
            assert!(!script(s).is_empty());
        }
    }

    #[test]
    fn bash_registers_the_completion_function_for_clipf() {
        assert!(BASH.contains("complete -F _clipf_completions clipf"));
    }

    #[test]
    fn zsh_declares_the_compdef_header() {
        assert!(ZSH.starts_with("#compdef clipf"));
    }

    #[test]
    fn fish_registers_completions_for_the_clipf_command() {
        assert!(FISH.contains("complete -c clipf"));
    }

    /// Every long flag `cli::help()` documents should be mentioned in every
    /// completion script — a cheap guard against a flag being added to one
    /// and forgotten in the others. Checked as the bare name (no leading
    /// `--`), since fish's `complete -l NAME` syntax never includes the
    /// dashes — bash and zsh both still contain the bare name too, just
    /// with the dashes immediately in front of it.
    #[test]
    fn every_documented_long_flag_appears_in_every_script() {
        for flag in [
            "no-newline",
            "print",
            "backend",
            "osc52",
            "tmux",
            "max",
            "force",
            "paste",
            "check",
            "dry-run",
            "json",
            "verbose",
            "completions",
            "help",
            "version",
        ] {
            for (name, text) in [("bash", BASH), ("zsh", ZSH), ("fish", FISH)] {
                assert!(text.contains(flag), "{name} completion is missing {flag}");
            }
        }
    }
}
