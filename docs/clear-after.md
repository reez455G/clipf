# `--clear-after SECONDS`: design note, not a plan

Status: **deferred**. This is a decision record, not a task list — nothing here
should be implemented without a follow-up decision that explicitly overrides
this document.

## The pitch

Clear the clipboard N seconds after copying a token/password, so a secret
doesn't sit there indefinitely for the next `Ctrl+V` (or the next process
that reads it — see the README's own "readable by other processes in this
session" framing, which D5's secret-pattern warning already leans on). This
is thematically exactly right for a tool that already goes out of its way
to self-wipe its memory buffers (`src/secret.rs`) and warn about obvious
key material (`src/scan.rs`). Several popular password-manager CLIs ship
this.

## Why it doesn't fit the current architecture

clipf is a single-shot process: it copies, prints at most a few lines, and
exits. `Cargo.toml`'s `[profile.release]` sets `panic = "abort"`, which is a
correctness invariant this directive explicitly protects — it also means a
clear-after timer can't just be "sleep, then clear, in the same process,"
because that would mean the process the shell is waiting on doesn't exit
for N seconds. That breaks the basic contract every example in the README
relies on: `ssh HOST 'cat FILE' | clipf` and every other pipeline assumes
clipf returns promptly.

So "clear after N seconds" necessarily means *something* outlives the
initial process. Three ways to get there:

## Option A — the main process just sleeps

`clipf token.txt --clear-after 30` copies, then blocks for 30 seconds
before exiting, then clears.

- **Cost:** none in binary size or complexity — it's a `std::thread::sleep`.
- **Why it's bad anyway:** the shell prompt doesn't return for 30 seconds.
  Nobody wants their terminal to hang after a copy. Rejected outright; not
  a real option, included only for completeness.

## Option B — double-fork daemon

clipf forks, the parent exits immediately (shell gets its prompt back), the
child detaches from the controlling terminal (`setsid`) and outlives the
parent, sleeps N seconds, clears, exits.

- **Cost:** the only `unsafe` this crate has today is `secret.rs`'s
  volatile-write wipe loop (justified: it's the entire value proposition).
  `fork()`/`setsid()` are libc calls with no std wrapper — this adds a
  second category of unsafe, on Unix only, doing process-model surgery
  (undefined interaction with anything else the parent had open: file
  descriptors, memory, whatever crt initialization already ran). Getting
  this right (avoiding zombie processes, correctly closing inherited fds,
  not deadlocking if the parent's stdout/stderr are pipes the child still
  holds open) is a well-known source of subtle bugs even in tools whose
  entire job is being a daemon.
- **Windows:** no `fork()` at all. The equivalent is spawning a *new*
  process with `CREATE_NO_WINDOW`/`DETACHED_PROCESS` creation flags via
  `CreateProcessW` — a different, unsafe, Windows-specific FFI surface,
  meaning this option is really two separate unsafe implementations, not
  one.
- **Binary size:** likely +5-15 KB for the platform-conditional process
  code, on top of a profile that currently ships at ~250 KB specifically
  *because* `opt-level = "z"` and `strip` are taken seriously (see the
  release workflow's size reporting). Modest, but not free, for a feature
  most invocations never touch.

## Option C — separate detached helper process

Rather than the main clipf binary forking itself, spawn something else to
own the delayed clear: `setsid nohup clipf --internal-clear-after 30 &`
via `std::process::Command` (no `unsafe` needed — this is just a normal
`Command::spawn()` with `Stdio::null()` on all three streams and the
process left undetached-but-abandoned), or delegate to the OS scheduler
where one exists (`systemd-run --on-active=30 ...` on systemd Linux,
`launchd`/`at` elsewhere) instead of clipf owning the wait at all.

- **Cost:** no new `unsafe`. Complexity moves into "does a suitable
  scheduler exist on this machine" detection, which is itself nontrivial
  and inherently platform-fragmented (systemd-run needs a user or system
  bus; `at` needs atd running and often needs to be enabled separately from
  install; Windows has Task Scheduler via `schtasks`, a third completely
  different mechanism). The self-spawn fallback (`Command::spawn` a
  detached copy of clipf) avoids the scheduler-detection problem but still
  needs clipf to grow a second, hidden mode (`--internal-clear-after`) that
  every packaging/audit story now has to account for — a background clipf
  process running unprompted is also just a surprising thing for a "copies
  a file and exits" tool to do without very explicit opt-in.

## Backend support matrix — the part that actually kills this for now

Even granting a working delayed-execution mechanism, "clear the clipboard"
only means something for backends clipf can *write to again later*:

| Backend | Can clipf clear it after the fact? |
|---|---|
| `xclip` / `xsel` | Yes — pipe an empty payload through the same command later. |
| `wl-copy` | Yes — `wl-copy --clear`, or an empty payload. |
| `pbcopy` | Yes — pipe empty input. |
| `clip.exe` | Yes — pipe empty input, same transcoding path as any other copy. |
| `termux-clipboard-set` | Yes — empty payload. |
| `osc52` | **No.** The terminal (and possibly a multiplexer) owns the clipboard once the escape sequence is sent; there is no "un-send" and no query-then-clear mechanism (the same reason `-O`/`--paste` doesn't work under OSC 52 either — see `src/backend.rs`'s `paste()` and D1's `PasteUnsupported`). |

OSC 52 is not a corner case here — it is described in the README's own
words as the reason this tool exists ("works over SSH with nothing
installed on the remote host"), and `check::report_json`'s own data shows
it's the *only* reachable backend on a plain headless SSH session with no
local helper tools. A `--clear-after` that silently does nothing for the
single most load-bearing backend would be actively misleading — worse than
not having the flag, because a user who set it would reasonably believe
their clipboard gets cleared and it wouldn't.

## Recommendation (not a decision — that's the point of deferring)

If this is picked up later: Option C without scheduler detection (a
self-spawned, `Command`-based detached instance of clipf itself in a new
hidden mode) is the least architecturally invasive — no new `unsafe`, no
per-OS fork semantics to get right, no scheduler-availability detection.
It should refuse to accept `--clear-after` when the resolved backend is
`osc52`, printing the same category of message `check::report_json` already
uses for other backend-shaped notes, rather than accepting the flag and
doing nothing.

Whatever gets built should ship with an explicit exit code or `--json`
field distinguishing "cleared" from "clear was scheduled but the backend
can't support it" — silent no-ops are exactly what this whole directive's
D1 (granular exit codes) and D3 (`--json`) exist to eliminate elsewhere in
this tool; a `--clear-after` that reintroduces one would be a regression
against the rest of 0.5.0, not just an unfinished feature.
