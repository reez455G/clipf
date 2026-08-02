---
name: clipf
description: Copy file contents or command output to the user's clipboard from a shell — locally, over SSH, in WSL, tmux/screen, or Android/Termux. Use whenever a task's end result is "put this text in the user's clipboard" (a config snippet, a generated token, log lines, command output) rather than just printing it to the terminal.
---

# clipf

`clipf` is a single static binary that copies stdin (or a file) to the
clipboard. It picks the right mechanism for the environment automatically —
local clipboard tool (`xclip`/`xsel`/`wl-copy`/`pbcopy`/`clip.exe`/
`termux-clipboard-set`) when one is available, or the OSC 52 terminal escape
sequence as a fallback that works over a plain SSH connection with nothing
installed on the remote host.

Use `clipf` instead of printing output and asking the user to select/copy it
by hand whenever the deliverable is meant to end up in their clipboard.

## Before using it

Check the binary is installed and confirm what backend it will pick in this
session:

```sh
command -v clipf || echo "not installed"
clipf --check      # shows OS, detected backend, and any environment warnings
```

If it's missing, install with:

```sh
curl -fsSL https://raw.githubusercontent.com/reez455G/clipf/main/install.sh | sh
```

or on native Windows (PowerShell): `irm https://raw.githubusercontent.com/reez455G/clipf/main/install.ps1 | iex`

## Core usage

```
clipf [OPTIONS] [FILE]
<command> | clipf [OPTIONS]
```

Reads `FILE`, or stdin when `FILE` is omitted or `-`. Copies **exactly one**
source per invocation — there is no `--last N` or `--grep` built in; slice the
input with the tool built for that and pipe the result in (see Recipes below).

```sh
clipf server.conf                     # copy a whole file
grep -v '^#' fw-rules.sh | clipf      # copy filtered/generated output
clipf -n token.txt                    # copy without the trailing newline
```

## Flags that matter for agent use

| Flag | When to reach for it |
|---|---|
| `-n`, `--no-newline` | Copying a token, hash, IP, or any value that must not have a trailing `\n` |
| `-v`, `--verbose` | Confirming which backend actually ran and how many bytes were copied — use this to verify the copy succeeded rather than assuming |
| `--dry-run` | Checking what *would* be copied (source, size, backend) without touching the clipboard — safe to use to preview before a real copy |
| `-p`, `--print` | Copying while also echoing the content to the terminal/log |
| `-f`, `--force` | Only when the payload is legitimately >64 KB and OSC 52 truncation risk is acceptable/expected |
| `-m BYTES`, `--max BYTES` | Raising or removing (`0`) the size guard for a known-large payload |
| `-O`, `--paste` | Reading the current clipboard back out to stdout — use to verify a copy round-tripped, or to consume something the user just copied |
| `-b NAME`, `--backend NAME` | Forcing a specific backend (`auto`, `osc52`, `xclip`, `xsel`, `wl`, `pbcopy`, `clip.exe`, `termux`) when auto-detection picked the wrong one |
| `--check` | Diagnosing "the copy didn't work" — always run this first when troubleshooting rather than guessing |

Full reference: `clipf --help`.

## Exit codes — check these, don't just check output

| Code | Meaning |
|---|---|
| `0` | copied successfully |
| `1` | usage error or the input file/backend command failed |
| `3` | refused: payload exceeds the OSC 52 size guard (64 KB by default) |

A `3` is not a fatal failure — it means the payload is large and OSC 52 would
silently truncate it. The fix is almost always to invert the direction (see
below), not to blindly pass `--force`.

## The one thing to get right over SSH

Inside an SSH session, `clipf` uses OSC 52 by default so the data lands on the
**local** machine's clipboard, not the remote server's. This is exactly what
you want when running `clipf` on a remote host on the user's behalf. But OSC
52 has a hard size ceiling (64 KB) and the terminal truncates overflow
**silently** — so for anything that might be large, run `clipf` on the local
side instead and pull the data over with an SSH command substitution:

```sh
ssh host 'cat /etc/openvpn/server.conf' | clipf
ssh host 'tail -n 200 /var/log/app.log' | clipf
```

This has no size limit and needs nothing installed on `host` at all.

## Recipes: let sed/awk/grep/tail do the slicing

`clipf` deliberately does not implement line ranges or filtering — compose it
with the standard tools instead:

```sh
tail -n 10 app.log | clipf                    # last 10 lines
head -n 20 app.log | clipf                    # first 20 lines
grep ERROR app.log | clipf                    # only matching lines
sed -n '10,20p' app.log | clipf               # a line range
awk '/START/,/END/' app.log | clipf           # everything between two markers
ssh host 'journalctl -u myapp -n 50' | grep ERROR | clipf
```

## Verifying a copy actually happened

Don't assume success from exit code alone in a scripted/agent context — a
`0` from OSC 52 only means the escape sequence was written, not that the
terminal accepted it (some terminals don't support OSC 52 at all).
Confirm with:

```sh
clipf -v somefile.txt      # stderr reports: source=... bytes=N backend=...
clipf -O                   # if the local backend supports paste, read it back
```

`-O`/`--paste` only works for backends that support reading (`xclip`, `xsel`,
`wl-paste`, `pbpaste`, `clip.exe`, `termux-clipboard-get`) — plain OSC 52
cannot be read back (terminals disable the query form as a security measure).
`clipf -O` under an OSC 52 backend exits `1` with an explicit error saying so;
`clipf --check` does not warn about this up front, only the paste attempt does.

## Repo

Source, install scripts, and full README: https://github.com/reez455G/clipf
