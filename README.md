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

## What it needs

**To build:** Rust 1.70 or newer. Nothing else — **zero dependencies**, std only.
base64, argument parsing and tty handling are all hand-rolled, so `cargo build`
works on an air-gapped machine and there is no supply chain to audit.

**To run:** nothing. A single binary, ~415 KB stripped.

**Optional, only for local desktop use:** `xclip`/`xsel` (X11), `wl-clipboard`
(Wayland). macOS and WSL already have `pbcopy`/`clip.exe`.

**For the SSH path:** nothing on the server, but the *local* terminal emulator
must speak OSC 52. `--check` will tell you whether yours does.

## Build

```sh
cargo build --release        # target/release/clipf
cargo test                   # 34 unit tests
```

### Deploying to CentOS 7

A binary built on a modern distro will **not** run on CentOS 7 — it links against
a newer glibc than 2.17. Build statically against musl instead:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

The result has no dynamic dependencies at all and runs on any x86-64 Linux,
CentOS 7 included. Verify with `ldd target/.../clipf` → "not a dynamic executable".

Pushing to a fleet:

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
reallocates and leaves an unwiped copy behind.

This is best-effort, not a guarantee: reading from stdin has unknown length so
growth can still leave stale copies, and nothing here defends against swap, core
dumps, or a process with ptrace rights. It is strictly better than a temp file,
not a secrets manager.

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

## tmux

```
set -g set-clipboard on
set -g allow-passthrough on   # only needed if you use --tmux
```

Existing panes keep the old setting — run `tmux kill-server` or start a fresh
session after changing it.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | copied |
| 1 | usage or file error |
| 3 | refused: exceeds the OSC 52 size guard |
