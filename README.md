# clipf (Rust)

Copy the contents of a file (or any piped output) to the clipboard — from a local
desktop or from inside an SSH session. Same CLI as the shell version, byte-identical
escape sequence output.

```
clipf server.conf
grep -v '^#' fw-rules.sh | clipf
clipf -n token.txt          # no trailing newline
clipf --check               # diagnose "why isn't this working"
```

## Install

One command, any Linux/macOS/WSL/Termux machine — downloads the prebuilt binary
for your platform, verifies its SHA-256, and puts it on your `PATH`:

```sh
curl -fsSL https://raw.githubusercontent.com/reez455G/clipf/main/install.sh | sh
```

Native Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/reez455G/clipf/main/install.ps1 | iex
```

From source, if you have Rust and would rather compile it:

```sh
cargo install --git https://github.com/reez455G/clipf --locked
```

**Not yet available: `cargo install clipf`.** The crate isn't published to
crates.io yet — publishing needs a `CARGO_REGISTRY_TOKEN` repo secret that
hasn't been configured. `.github/workflows/release.yml`'s `publish-crate`
job is wired to run `cargo publish --locked` automatically on the next
tagged release once that secret exists; until then, use one of the methods
above.

`install.sh` flags: `--version v0.4.0` to pin a release, `--bin-dir DIR` to choose
where it lands (default `/usr/local/bin` as root, `$PREFIX/bin` on Termux,
`~/.local/bin` otherwise), `--no-path` to leave your shell rc files alone, and
`--build` to compile from source instead of downloading. `CLIPF_VERSION` and
`CLIPF_BIN_DIR` work as environment equivalents.

### Deploy to a remote host

```sh
./install.sh --remote user@host
```

The binary is downloaded and checksum-verified **locally**, then copied over the
SSH connection you already have. The remote host needs no internet access, no
compiler, and no clipboard tooling — the Linux builds are static musl binaries,
so they run on anything from CentOS 7 to Alpine.

### Platform support

| Platform | Prebuilt | Default backend |
|---|---|---|
| Linux x86_64 / aarch64 | yes (static musl) | `wl-copy`/`xclip`/`xsel` on a desktop, OSC 52 headless |
| macOS x86_64 / arm64 | yes | `pbcopy` |
| Windows x86_64 | yes | `clip.exe` |
| WSL | use the Linux x86_64 build | `clip.exe` |
| Android / Termux | use the Linux build for your device | `termux-clipboard-set` |
| Other Unix (BSD, illumos, …) | build from source | OSC 52, or `xclip`/`xsel` |

### Shell completions

Bash, zsh, and fish completions ship attached to every release
(`clipf.bash`/`clipf.zsh`/`clipf.fish`), generated at release time — no
compiler or clipf install needed just to get them. Or generate them
yourself from an installed `clipf`:

```sh
clipf --completions bash | sudo tee /etc/bash_completion.d/clipf
clipf --completions zsh > "${fpath[1]}/_clipf"    # any dir on your $fpath
clipf --completions fish > ~/.config/fish/completions/clipf.fish
```

## What it needs

**To build:** Rust 1.70 or newer. Nothing else — **zero dependencies**, std only.
base64, argument parsing and tty handling are all hand-rolled, so `cargo build`
works on an air-gapped machine and there is no supply chain to audit.

**To run:** nothing. A single binary, ~415 KB stripped.

**Optional, only for local desktop use:** `xclip`/`xsel` (X11), `wl-clipboard`
(Wayland), `termux-clipboard-set` (Android/Termux). macOS and WSL already have
`pbcopy`/`clip.exe`.

**For the SSH path:** nothing on the server, but the *local* terminal emulator
must speak OSC 52. `--check` will tell you whether yours does.

## Build

```sh
cargo build --release        # target/release/clipf
cargo test                   # 37 unit tests
```

### Building it yourself

The published Linux assets are already static musl binaries, so old distros
(CentOS 7 and its glibc 2.17 included) work straight from the installer above.
To reproduce that build locally:
`rustup target add x86_64-unknown-linux-musl && cargo build --release --target x86_64-unknown-linux-musl`
— the result has no dynamic dependencies at all (`ldd` says "not a dynamic
executable").

Pushing to a fleet, without the installer:

```yaml
- name: install clipf
  copy:
    src: target/x86_64-unknown-linux-musl/release/clipf
    dest: /usr/local/bin/clipf
    mode: '0755'
```

## Layout

| File | Role |
|---|---|
| `src/main.rs` | input reading, dispatch, size guard |
| `src/cli.rs` | argument parsing, help text |
| `src/base64.rs` | RFC 4648 encoder (+ decoder used by tests) |
| `src/backend.rs` | backend enum, auto-detection, local helper tools |
| `src/osc52.rs` | escape sequence construction, tmux/screen wrapping |
| `src/term.rs` | tty opening, multiplexer and emulator detection |
| `src/secret.rs` | self-wiping byte buffer |
| `src/check.rs` | `--check` diagnostics |

## What this version does that the shell version doesn't

**Secrets never touch the disk.** The shell version staged input in a temp file
under `/tmp` — meaning `.env` contents and private keys hit the filesystem, where
they survive a crash and may outlive the process. Here the payload stays in
memory, in a buffer that overwrites itself on drop with volatile writes the
optimiser cannot elide. File reads preallocate the exact size so the buffer never
reallocates; stdin reads (unknown length up front) are read in fixed-size
chunks that also never reallocate, concatenated into one exact-size buffer
once the total is known — so, as of 0.5.0, neither path leaves a stale,
unwiped copy behind from a buffer growing mid-read.

This is still best-effort, not a guarantee: nothing here defends against the
OS paging a page out to swap, a core dump, or a process with ptrace rights.
It is strictly better than a temp file, not a secrets manager.

**screen payloads are chunked correctly.** GNU screen truncates long DCS
passthrough strings. The shell version emitted one oversized DCS and silently
lost data; this one splits the sequence into 448-byte chunks that screen
reassembles. Verified in tests by reassembling a 5000-byte payload.

**`--check` identifies your actual terminal.** It reads emulator-specific
variables (`KITTY_WINDOW_ID`, `WEZTERM_PANE`, `WT_SESSION`, `TERM_PROGRAM`, …)
rather than trusting `TERM`, which is `xterm-256color` on almost everything, and
gives a verdict: supported, not supported, or unknown.

**No subprocesses on the OSC 52 path.** No `base64`, no `tr`, no temp file — one
`open()` and one `write()`. It also means clipf works on a host where coreutils
is missing or broken.

**Real error handling.** Distinct messages for missing file, directory,
permission denied, and missing helper binary, with distinct exit codes.

## The size limit, and when to avoid OSC 52

OSC 52 pushes the whole file through the terminal as one escape sequence.
Terminals and tmux cap that, and they **truncate silently** — you get a partial
file with no error. clipf refuses above 64 KB by default (exit 3) and reports both
the raw and base64-encoded size:

```
$ clipf big-config.conf
clipf: 1048576 bytes (1398104 once base64-encoded) exceeds the OSC 52 guard of 65536 bytes.
clipf: Terminals truncate oversized payloads without reporting an error,
clipf: so this would most likely copy a partial file. Options:
clipf:   - run from your local shell:  ssh HOST 'cat big-config.conf' | clipf
clipf:   - raise the cap:              clipf --max 0 --force big-config.conf
```

For anything large, invert the direction and run from your local shell:

```sh
ssh ovpn1 'cat /etc/openvpn/server.conf' | clipf
```

No size limit, no terminal support needed, faster.

## Recipes: combining with sed/awk/grep/tail

clipf only does one thing — put bytes on the clipboard, safely, on whatever
platform you're on. It deliberately has no `--last N`, `--grep PATTERN`, or
`--between` flags: `tail`, `grep`, `sed` and `awk` already do that job better,
are on every machine, and compose freely through a pipe. Slicing the input is
not clipf's problem to solve.

```sh
tail -n 10 app.log | clipf                    # last 10 lines
head -n 20 app.log | clipf                    # first 20 lines
grep ERROR app.log | clipf                     # only matching lines
sed -n '10,20p' app.log | clipf                # a line range
awk '/START/,/END/' app.log | clipf            # everything between two markers

# these compose over SSH exactly the same way — the pipeline runs on the
# remote host, only the final bytes cross the wire to your local clipboard:
ssh host 'tail -n 10 /var/log/app.log' | clipf
ssh host 'journalctl -u myapp -n 50' | grep ERROR | clipf
```

## tmux

```
set -g set-clipboard on
set -g allow-passthrough on   # only needed if you use --tmux
```

Existing panes keep the old setting — run `tmux kill-server` or start a fresh
session after changing it.

## Exit codes

**Changed in 0.5.0:** exit codes are now granular — file errors used to
collapse into `1` along with usage errors, and `--paste` under OSC 52 used to
return `1` too. Scripts branching on exact codes should be updated; scripts
that only checked "zero or non-zero" are unaffected.

| Code | Meaning |
|---|---|
| 0 | copied (or `--dry-run`/`--check`/`--help`/`--version` completed) |
| 1 | usage error: unknown flag, missing/malformed flag value, conflicting flags, or nothing to read (no FILE, stdin is a terminal) |
| 3 | refused: payload exceeds the OSC 52 size guard |
| 4 | input error: file missing, is a directory, permission denied, or a read failed |
| 5 | backend unavailable: helper binary not found, or the backend has no local helper at all (`osc52`) |
| 6 | backend failed: helper spawned but exited non-zero, or a write into the pipeline failed |
| 7 | reserved, not currently reachable — see `src/exit.rs` |
| 8 | `-O`/`--paste` against a backend that cannot be read back (OSC 52) |

## Machine-readable output (`--json`)

New in 0.5.0. Writes a single JSON object to stdout instead of (or on top of)
the usual prose, for scripts and agents that need to branch on structured
fields rather than parse stderr text. Not combinable with `-p`/`--print` or
`-O`/`--paste` — both already write their own payload to stdout, and `--json`
is rejected at argument-parsing time if either is also given.

```sh
clipf --check --json
clipf --json server.conf         # copy, reporting what happened
clipf --json --dry-run server.conf
```

`--check --json`:

```json
{
  "schema": 1,
  "clipf": "0.5.0",
  "os": "linux",
  "backend": { "selected": "osc52", "available": ["osc52"], "source": "auto" },
  "multiplexer": null,
  "emulator": null,
  "osc52": "unknown",
  "tty": "stderr",
  "ssh": false,
  "max_bytes": 65536,
  "warnings": [ { "code": "...", "message": "..." } ]
}
```

`--json` on a copy (add `-v` too if you also want the stderr prose):

```json
{ "schema": 1, "clipf": "0.5.0", "ok": true, "source": "server.conf",
  "bytes": 1234, "encoded_bytes": 1648, "backend": "osc52",
  "stripped_newline": false, "dry_run": false }
```

On failure `ok` is `false` and an `error` object appears:
`{ "code": 4, "kind": "input", "message": "no such file: x.txt" }` — `code`
matches the process exit code, `kind` is the machine-stable name from the
table above (`usage`, `too_big`, `input`, `backend_unavailable`,
`backend_failed`, `paste_unsupported`). `bytes`/`encoded_bytes`/`backend` are
`null` when the failure happened before they were known — except for the
`too_big` refusal (exit `3`), where they're always present, since that's
exactly the case where knowing the size is useful even though nothing was
copied.

**Stability contract:** `schema` only increments on a breaking change to an
*existing* field's meaning or type, or a field's removal. Adding a new field
does not bump it — code that reads this JSON should ignore keys it doesn't
recognise rather than reject the object outright.
