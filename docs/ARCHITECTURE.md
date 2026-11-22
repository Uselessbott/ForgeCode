# ForgeBox — Milestone 1 Technical Architecture

Status: **MILESTONE 1 ONLY — architecture and risk assessment. No application code has been written yet.**

Date: 2026-11-22

---

## 0. Scope, method, and non-claims

This document is the deliverable for Milestone 1 of the ForgeBox project. It defines the architecture for a **single Android application** that combines a professional code editor and a native, integrated Linux terminal.

Facts that constrain the design were checked against current platform documentation and upstream project documentation:

| Claim | Verified basis |
|---|---|
| Play requires `targetSdk 36` for new apps/updates since 2026-08-31 | [Play Console Help](https://support.google.com/googleplay/android-developer/answer/11926878?hl=en) |
| Apps targeting API 29+ cannot `execve()` files in their writable app home directory (W^X / SELinux `app_data_file`) | [Android 10 behavior changes](https://developer.android.com/about/versions/10/behavior-changes-10), [Termux execution environment](https://github.com/termux/termux-packages/wiki/Termux-Execution-Environment) |
| Bionic provides `openpty`, `forkpty`, `posix_openpt` since API 23 | [Bionic pty.h](https://android.googlesource.com/platform/bionic/+/main/libc/include/pty.h), [Bionic pty.cpp](https://android.googlesource.com/platform/bionic/+/main/libc/bionic/pty.cpp) |
| `isolatedProcess` gives a different UID and SELinux domain, but no permissions, no socket creation, no direct filesystem/network access, and cannot `PROT_EXEC` map app files | [service element](https://developer.android.com/guide/topics/manifest/service-element), [AOSP isolated_app.te](https://android.googlesource.com/platform/system/sepolicy/+/fa60d7fc6081cef168804c0bf574ae507c803300/private/isolated_app.te), [Chromium Android sandbox design](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/security/android-sandbox.md), [Termux issue #1072 test results](https://github.com/termux/termux-app/issues/1072) |
| PRoot is ptrace-based path translation, not a security boundary; no real root, no namespaces/cgroups, no init systems, slow syscall-heavy workloads | [PRoot](https://proot-me.github.io/), [Termux Wiki](https://wiki.termux.com/wiki/PRoot), [proot-distro limitations](https://github.com/termux/proot-distro#limitations) |
| A patched, Android-16-compatible PRoot engine with an `:proot-engine` AAR and Rust CLI already exists (GPL-2.0 proot) | [oonid/pr](https://github.com/oonid/pr) |
| sora-editor is a mature Android code editor widget with tree-sitter support, LGPL-2.1 | [sora-editor](https://github.com/Rosemoe/sora-editor), [docs](https://project-sora.github.io/sora-editor-docs/guide/editor-overview) |
| `git2` can vendor libgit2 and OpenSSL for Android builds | [git2-rs](https://github.com/rust-lang/git2-rs), [libgit2 Android notes](https://github.com/libgit2/libgit2) |
| Android 12+ monitors/kills app child processes (default ceiling ~32 system-wide) | [Phantom processes analysis](https://raw.githubusercontent.com/agnostic-apollo/Android-Docs/master/en/docs/apps/processes/phantom-cached-and-empty-processes.md) |

**We explicitly do not claim:**

1. That shell processes forked by ForgeBox are isolated from the app's own private files. They share the app UID; Linux DAC does not separate them.
2. That PRoot is a sandbox. It is a compatibility layer.
3. That an `isolatedProcess` can run a shell, open files directly, or create sockets. It cannot; it is only useful here for narrow, FD-brokerable data-parsing tasks.
4. That stock Android offers per-process network filtering via a public API. `VpnService` is UID/package scoped. Per-process network control is only achievable with a syscall filter on the child, which must be validated on real devices (see R-04).
5. That desktop OpenCode / Codex CLI binaries will run on stock Android. They will not, without a compatible userspace and their runtime dependencies.
6. That full desktop Android Studio functionality (Gradle emits, SDK emulators, AVD) can run on a phone. Remote execution is the documented fallback.

---

## 1. Overall application architecture

### 1.1 Layered view

```
┌───────────────────────────────────────────────────────────────────────┐
│ L4  PRESENTATION (Kotlin / Jetpack Compose)                           │
│  EditorHost (AndroidView of sora-editor) · TerminalCanvas · Explorer  │
│  SourceControlPanel · ApprovalSheet · NetworkMonitor · BuildPanel     │
│  Phone compact layout / Tablet multi-pane layout · ProgrammingToolbar │
└───────────────────────────────┬───────────────────────────────────────┘
                                │ narrow JNI façade (protobuf messages, int handles)
┌───────────────────────────────▼───────────────────────────────────────┐
│ L3  RUST CORE  (single cdylib: libforgebox_core.so, loaded by app)    │
│  workspace · vfs · pathguard · index/search · document · editor_core  │
│  git · policy · supervisor · pty/session · terminal (vte) · checkpoint │
│  archive · audit · config · agent_session · net_policy                │
└──────┬──────────────────┬────────────────────┬────────────────────────┘
       │ POSIX/FFI        │ JNI callbacks      │ local sockets
┌──────▼────────┐  ┌──────▼──────────┐  ┌──────▼────────────────────────┐
│ Child procs   │  │ Kotlin platform │  │ Brokers (Kotlin, Keystore)    │
│ sh/PRoot/tools│  │ SAF/Keystore/   │  │ secrets · git auth · network  │
│ PTY slave fds │  │ FGS/network API │  │ source-upload consent         │
└───────────────┘  └─────────────────┘  └───────────────────────────────┘
```

### 1.2 Process model

| Process | UID / SELinux | Purpose | Can it execute a shell? | Can it open network sockets? |
|---|---|---|---|---|
| Main app process (`com.forgebox`) | app UID / `untrusted_app` | Compose UI, Rust core, broker modules, foreground service | Yes, via Rust `forkpty` + `exec` | Yes (manifest `INTERNET`, policy enforced in broker) |
| Child PTY processes (shell, tools, agents, PRoot tracee) | **same app UID** / `untrusted_app` | Real command execution | Yes | Yes at OS level; policy attempt via seccomp (R-04) |
| `:secure_parser` isolated service | **isolated UID** / `isolated_app` | Parsing untrusted diffs, archives metadata, agent output where FD-only input suffices | No | No — communicates through Binder with passed FDs |
| Remote backend (optional) | external | Heavy builds, full agent CLIs | Yes | User-configured SSH |

**Consequence:** the untrusted shell/agent tier is separated from the UI tier only by *process boundaries*, not by UID or MAC. All security that matters for that tier must be enforced at the syscall/tool-gateway layer and by ensuring no secret is present in any file readable by the app UID.

### 1.3 Editor ↔ terminal shared-file data flow

```
Terminal: python main.py  ──writes──▶  Projects/P/main.py
                                        │
                        inotify (Rust) ─┤  (fallback: 1 s stat scan)
                                        ▼
                            Rust workspace/index
                                        │  change event over JNI
                                        ▼
                  Compose: if buffer clean → reload text, preserve cursor
                           if buffer dirty → show "file changed on disk" banner
                                             with Diff / Reload / Keep mine

Editor keystroke ─▶ sora Content edit event ─▶ JNI buffer mirror (Rust rope)
                 ─▶ autosave journal (fsync) ─▶ atomic rename onto project file
                 ─▶ inotify event ─▶ terminal sees updated file
```

This is a **single shared workspace path on disk**, not two copies and not a simulated terminal.

---

## 2. Rust / Kotlin division of responsibilities

### 2.1 Rust owns

- Workspace root resolution, project registry, virtual-path validation, traversal/symlink defence (single choke point: `vfs::open`).
- All file CRUD used by explorer/editor/agent tools, with audit records.
- Document mirror (`ropey`), incremental search/replace, regex search, index of symbols/folds, tree-sitter parse trees for indexing and navigation.
- Terminal emulation (`vte` parser + grid + scrollback), PTY lifecycle, process groups, signals, resource limits, session registry.
- Command policy engine, destructive-operation classifier, approval token issuance.
- Git operations via `git2` (status, diff, stage, commit, branch, history, local clone).
- Checkpoint store, content-addressed snapshots, diff/restore, autosave journal replay.
- Archive inspection/extraction with ZIP-slip, symlink, bomb and size limits.
- Network *policy decisions* (allow/deny/ask) and traffic accounting; actual sockets in Kotlin broker.
- Agent session state machine, tool-call registry, cancellation tokens, file-change tracking.

### 2.2 Kotlin owns

- Android lifecycle, `Application`, activities, foreground service + notification, wake lock.
- Jetpack Compose UI, Material 3, window size classes, IME insets, touch gestures, Bluetooth keyboard event mapping.
- sora-editor `CodeEditor` host (AndroidView) and its language/theme configuration.
- Android permissions, SAF `ACTION_OPEN_DOCUMENT` / `ACTION_CREATE_DOCUMENT` / tree URIs, `DocumentFile`.
- Android Keystore + `EncryptedSharedPreferences` / BiometricPrompt.
- OkHttp + `ConnectivityManager` + `Network.bindSocket` network broker.
- GitHub OAuth device-flow browser/intent handling; token never handed to child processes.
- AIDL contract for the `:secure_parser` isolated service.
- Notifications, `VpnService` evaluation (not used for per-process filtering in v1).

### 2.3 Rationale

Anything involving a permission, an Android Binder API, an IME, or a lifecycle callback stays in Kotlin. Everything else is deterministic, testable Rust with the same code running in host unit tests and on-device.

---

## 3. JNI design

### 3.1 Shape of the interface

One native singleton per app process, created at `Application.onCreate`:

```kotlin
object ForgeNative {
    init { System.loadLibrary("forgebox_core") }
    external fun init(appDataDir: String, nativeLibDir: String, dbPath: String): Long
    external fun call(runtime: Long, request: ByteBuffer): ByteBuffer
    external fun pollEvents(runtime: Long, max: Int): List<ByteArray>
    external fun shutdown(runtime: Long)
}
```

- `call` uses **length-prefixed protobuf** (`prost`) requests/responses. One schema file (`forgebox.proto`) is the contract. This gives version tolerance and avoids dozens of fragile JNI signatures.
- `pollEvents` returns serialized events (`FileChanged`, `TerminalOutput`, `ApprovalRequired`, `AgentToolCall`, `NetworkRequest`, `BuildDiagnostic`, `CheckpointCreated`). A dedicated native thread holds a global ref to a Kotlin `EventSink` and pushes events; the Compose layer collects them into a `Channel`/`StateFlow`.
- **Hot path exception:** editor text editing uses direct methods (`nativeEdit(bufferId, start, end, utf8)`) with `DirectByteBuffer` snapshots. No protobuf allocation per keystroke.
- Paths cross JNI as Java strings decoded once into Rust `PathBuf` and immediately validated by `pathguard`.
- Opaque integer handles (buffer/session/project/checkpoint) — no Rust pointers in Kotlin.
- Every JNI entry point is wrapped in `catch_unwind`; panics become `Status::InternalError` with a stable error code, never a process crash.
- Calling threads are annotated: UI calls are short; file/PTY I/O runs on Rust `tokio` workers; blocking calls are never issued from the Compose main thread.
- `JNI_OnLoad` caches `JavaVM`; native threads attach/detach explicitly.

### 3.2 Backpressure and lifetime

- Terminal output is coalesced in Rust into 16 ms / 64 KiB batches before crossing JNI.
- Scrollback is ring-buffered in Rust (default 10 000 lines, configurable); Compose renders only visible rows.
- On `onTerminate`/task removal the app calls `shutdown`, which signals all process groups, flushes journals/audit, and closes FDs. A foreground service keeps sessions alive during normal app backgrounding.

---

## 4. Editor technology

### 4.1 Decision

**Use sora-editor (LGPL-2.1) as the editing widget, hosted in Compose via `AndroidView`, with `language-treesitter` for incremental highlighting and folding; keep a Rust `editor-core` mirror for persistence, search, indexing, checkpoints and crash recovery.**

Alternatives considered:

| Option | Verdict |
|---|---|
| Compose `BasicTextField` | Rejected. No line numbers, folding, bracket matching, IME cursor control, or large-file performance. |
| Build a Compose Canvas editor from scratch with Rust rope/tree-sitter | Rejected for v1. IME composition, selection handles, magnifier, word wrap and hardware-key mapping are months of work and would be inferior to sora-editor on touch. |
| WebView + CodeMirror 6 | Viable fallback, but weaker native keyboard/IME control and a second runtime; not chosen. |
| sora-editor + Rust mirror | **Chosen.** Mature touch editor, mature tree-sitter integration, Rust owns all durable state and security-relevant logic. |

### 4.2 Source-of-truth rule

- While a file is open, **sora-editor is authoritative for live user edits and its own undo stack**.
- Rust mirrors every change into a rope, updates tree-sitter incrementally, updates the index, and appends to the crash-recovery journal. Rust never silently rewrites the widget's text.
- On external change (terminal/agent): if the buffer is clean, reload; if dirty, show a diff-based conflict UI. Never auto-overwrite unsaved work.
- Rust owns undo for *tool/agent* edits and for recovery replay; this is a separate, explicitly labelled history from the widget's user-edit undo.

### 4.3 Feature mapping

| Requirement | Implementation |
|---|---|
| Syntax highlighting, folding, bracket match/completion, auto-indent, undo/redo, search/replace | sora-editor core + tree-sitter queries; Rust mirrors and validates |
| Multiple tabs, split panes | Compose tab model; sora-editor instance per pane; tab state persisted in SQLite |
| Multi-cursor | sora-editor selection model + Compose key/toolbar bindings; verify coverage per version and extend if needed |
| Find in files / replace in files / regex / case / whole word | Rust `index` + `regex` crate running over workspace with ignore rules; results streamed to Compose |
| Go to line / symbol / definition / rename | Line: editor native. Symbol: tree-sitter tag queries in Rust. Definition/rename: LSP via remote/local language servers where available (best-effort, not promised for all 18 languages) |
| Formatting | Formatter from configured toolchain (e.g. `rustfmt`, `black`) invoked through policy engine; fallback: none, with UI explanation |
| Autosave / crash recovery / tab restore | Rust append-only journal + periodic atomic snapshots + SQLite tab/cursor state |
| Large files / incremental parsing | sora-editor + tree-sitter incremental parse; Rust rope; line-range virtualisation; hard limits with explicit warning over configurable size |
| UTF-8, LF/CRLF | Rust normalises for indexing while preserving original EOL on write; sora-editor configured for UTF-8 |
| Programming keyboard toolbar | Compose row of keys emitting sora-editor key events: Esc, Tab, Ctrl, Alt, Shift, Home, End, Del, arrows, brackets, quotes, operators, `/`, `\`; sticky modifier state; physical keyboard pass-through |

Languages: the 18 requested grammars are packaged as tree-sitter grammars + highlight/fold/bracket queries. Missing Android-grammar builds are cross-compiled with the NDK and vendored (the upstream `android-tree-sitter` project is unmaintained, so budget for maintaining a fork or moving to a current binding, see R-02).

---

## 5. Terminal and PTY implementation

### 5.1 PTY

Rust FFI to Bionic: `posix_openpt(O_RDWR|O_NOCTTY)` → `grantpt` → `unlockpt` → `ptsname_r` → `open(slave)`; child calls `setsid()`, `ioctl(TIOCSCTTY)`, `dup2` of the slave to fd 0/1/2. Resize via `TIOCSWINSZ`. These symbols exist since API 23 ([Bionic](https://android.googlesource.com/platform/bionic/+/main/libc/include/pty.h)), so PTYs are viable without root.

### 5.2 Emulation and rendering

- Rust uses the `vte` crate to parse the PTY byte stream into a `TerminalState` (grid, attributes, colors, cursor, modes, OSC52 clipboard, alt screen).
- Rust emits **screen diffs** to Compose. Compose renders monospace cells with `Canvas` + `TextMeasurer`, handles touch selection, scrollback gestures and IME input.
- Scrollback is a ring buffer in Rust, persisted per session (metadata + visible title + cwd; full scrollback optional due to size).
- Ctrl+C / Ctrl+D / Ctrl+Z are sent as terminal control bytes (`0x03`, `0x04`, `0x1A`); the supervisor also exposes explicit `SIGINT`/`SIGTERM`/`SIGKILL` to the process group via `killpg`. Ctrl+Z may be ignored by some Android toolchains; this is documented, not simulated.

### 5.3 What actually executes

| Tier | Available in | Notes |
|---|---|---|
| `/system/bin/sh` (mksh) + toybox applets | Every supported device, from M6 | Real shell and real coreutils, no extra install. Not bash; POSIX-ish. |
| Bionic-linked helper binaries shipped in `lib/<abi>/` as `lib*.so`, `extractNativeLibs=true` | From M6/M12 | W^X-safe because installed from signed APK. Used for git/portable utilities where system toybox is insufficient. |
| PRoot + Debian/Ubuntu/Alpine rootfs | Optional, from M7/M12 | glibc toolchains, `apt`, full Bash, GCC, Rust, Node (ARM64). **Compatibility layer, not a security boundary.** |
| SSH remote session | Optional, from M10/M12 | Real full toolchains and agent CLIs on a remote host; local UI only. |

`python main.py` works when Python is provided by a runtime pack or guest userspace. Scripts in the project are read as data by an interpreter located in `nativeLibraryDir` or the guest; we never rely on `execve` of a file in the writable project directory, which Android blocks.

---

## 6. Linux userspace selection

| Candidate | Pros | Cons | Decision |
|---|---|---|---|
| Stock Android `/system/bin/sh` + toybox | Zero download, real execution, smallest attack surface | Limited tools, no git/python/bash | **Baseline, ship in M6** |
| APK-packaged Bionic binaries | W^X compliant, signed, predictable | Must be cross-compiled and named `lib*.so`; no hot updates; licensing review | **Selective packaging for git/tools** |
| PRoot + distro rootfs | Real Bash, apt, Python, Node, gcc, cargo; widely used | ptrace overhead, no real root/namespaces/cgroups/init, SELinux/seccomp quirks; GPL-2.0/3.0 obligations | **Optional backend.** Strongly consider integrating the existing `:proot-engine` AAR from `oonid/pr` (patched proot, `PROOT_LOADER` in nativeLibraryDir, SIGSYS emulation, Rust CLI, 330+ host tests) instead of writing our own |
| Android Virtualization Framework non-protected VM | Genuine isolation in principle | Availability and public API for arbitrary Linux guests are not dependable on target phones in 2026; not a v1 dependency | Research item only |
| Remote SSH | Full desktop toolchains and agent CLIs | Requires network and a remote host; user-configured | **Documented fallback for unsupported workloads** |

PRoot is presented in the UI as "Linux compatibility environment (not a security sandbox)". Users must not be allowed to believe otherwise.

---

## 7. Shared workspace abstraction

### 7.1 On-disk layout

```
<app-private>/
├── ForgeBox/
│   ├── Projects/<project-id>/       # user code; visible to shell & agents
│   ├── Temporary/
│   ├── Imports/
│   ├── Exports/
│   ├── Backups/
│   ├── Cache/
│   ├── Config/                      # non-secret settings
│   └── AgentSessions/
└── internal/                        # brokered; never exposed as a project
    ├── db/forgebox.sqlite
    ├── audit/audit.jsonl
    ├── checkpoints/<id>/...
    ├── journal/<buffer-id>.wal
    └── secrets/                     # Keystore-wrapped blobs only, never plaintext tokens
```

### 7.2 Enforcement

- All access goes through `vfs`: resolve logical project path → canonicalize with `openat2`-style checks where available, otherwise `realpath` + component-by-component `symlink_metadata` check → verify result is under the assigned project root → then open with `O_NOFOLLOW` on the final component.
- Reject absolute paths, `..`, NUL, backslash tricks, case-confusion on case-insensitive filesystems, and any symlink whose target leaves the root.
- Shell sessions start with `cwd = Projects/<id>`, `HOME = <project>/.forgebox-home`, a scrubbed environment, and `PATH` limited to approved tool locations.
- PRoot binds the project at the **same absolute path** inside the guest as outside, so editor and terminal references never diverge.
- `inotify` watchers on the project tree detect external writes; a 1 s stat-scan fallback covers watcher failure and some vendor kernels.
- Explorer operations (create/rename/delete/copy/cut/paste/move/multi-select) are Rust `vfs` operations with audit records and checkpoint hooks.

---

## 8. Agent execution restrictions

### 8.1 Threat model summary

| Threat | Mitigation | Residual risk |
|---|---|---|
| Agent deletes personal files | No unrestricted path input: agent tool API accepts project-relative paths only, validated by `vfs`; commands classified by policy | Same-UID shell can still reach any app-readable file if it escapes the tool API — see below |
| Agent reads API keys | Keys live only in Android Keystore; decrypted values exist only in the Kotlin broker process; never in child env, git config, project files, or logs | A compromised broker process would expose them; broker is small, audited code |
| Agent runs destructive git command | Policy engine requires user confirmation; no agent self-approval | A shell can invoke a different git binary; terminal policy is stricter for agent sessions |
| Agent runs forever / forks bomb | Per-session process group, `setrlimit` (CPU/AS/FSIZE/NPROC), wall-clock timeout, `killpg`, global **Stop All** | Android 12+ phantom-process killer can still kill children; documented |
| Agent modifies files without recovery | Mandatory pre-edit checkpoint + reviewable diff + restore | Disk exhaustion limits checkpoint count |
| Malicious git hooks | `core.hooksPath` pointed at an empty controlled directory for agent operations | User-initiated terminal git may still run hooks; confirmed in UI |
| Credential leakage via output | Output redaction patterns in Rust before Compose and before logs; token never passed to children | Best-effort, not proof |
| Archive attacks | Two-pass archive inspection, limits, no symlink/device entries, extraction into fresh empty dir under `Imports/` | New archive formats need rule updates |

### 8.2 Isolation architecture — honest position

Within one stock Android app, a forked shell shares the app UID, so **no DAC isolation exists between the shell and ForgeBox's own files**. PRoot does not change this. Therefore:

1. **Do not store decryptable secrets in any file.** Keystore holds the key; broker holds plaintext in memory only. A hostile child can read an encrypted blob but cannot decrypt it.
2. **Do not give the child credentials via environment variables or arguments.**
3. **Expose agents to a capability-based tool gateway, not a raw filesystem handle.** Agent CLI integrations use a restricted JSON-RPC over stdio/local socket with tools: `list`, `read`, `write`, `search`, `diff`, `checkpoint`, `run_checked_command`. Each call is authorised by Rust `policy`.
4. **Apply a stacking seccomp-BPF filter to child processes** (after `PR_SET_NO_NEW_PRIVS`) to remove `ptrace`, `mount`, `setuid`, and, where validated, `socket`/`connect` for agent sessions, forcing network through the local broker. Android's zygote filter already applies; filters stack and can only narrow. This must be proven on target devices — see R-04. If it fails, we degrade honestly: no per-process network claim, explicit per-session network toggle with a warning banner.
5. **`:secure_parser` isolated service** is used for parsing untrusted agent output/diffs/archive metadata where input can be passed as an FD over Binder. It has a different UID and `isolated_app` SELinux domain, no sockets and no direct file access. It is deliberately *not* used for code execution because it cannot exec or map executable app files.
6. **Remote agent sessions** (SSH) are the supported path for OpenCode/Codex CLI if local execution is impractical. They are clearly labelled as remote; files are not uploaded without consent.

### 8.3 Approval model

- Policy result is `ALLOW`, `CONFIRM`, or `DENY`.
- `CONFIRM` shows: full resolved command, parsed argv, working directory, affected project, file count, explanation of effects, whether network is used, and whether a checkpoint will be created.
- Confirmation is a UI action by the human (optionally BiometricPrompt). An agent token can never satisfy a `CONFIRM`.
- Confirmation tokens are single-use, time-limited (≤60 s), and bound to one command hash and one session.
- Stop All terminates every process group, closes PTYs, cancels agent RPCs, and marks sessions as stopped.

---

## 9. Destructive-operation prevention and recovery

### 9.1 Confirmed operations

Recursive delete; project deletion; bulk overwrite above threshold; `git push --force*`; `git reset --hard`; history rewriting (`filter-branch`, `filter-repo`, `rebase -i` in non-interactive form); hook changes; auth setting changes; writes to `Config/`, `internal/`, or outside the project; `dd`, `mkfs*`, `shred`, `chmod/chown -R`, `mount`; any detected shell escape (`;`, `&&`, backticks, `$()`, pipelines to interpreters) in an agent-supplied command.

### 9.2 Implementation

- Static shell-word parser in Rust (not regex-only) with resolver of aliases/functions defined in the session and detection of common obfuscation. Unknown constructs default to `CONFIRM`.
- Every `CONFIRM` creates a checkpoint before execution.
- Append-only audit: timestamp, actor (user/agent id/session), operation, resolved paths, before/after hashes, policy result, approval id.
- Checkpoints are content-addressed snapshots plus a manifest; restore verifies hashes before overwriting. Last N checkpoints retained; project backup before bulk operations.
- Recovery UI lists checkpoints with diffs and one-tap Restore / Discard agent changes.
- Nothing pushes to GitHub automatically, ever.

---

## 10. Android security limitations (must be restated in product docs)

1. No root; no `MANAGE_EXTERNAL_STORAGE`; no `mount`, real `chroot`, namespaces or cgroups.
2. Same-UID child processes can read the app's private files. Workspace isolation is a **Rust-enforced application boundary**, not a kernel boundary.
3. PRoot is a compatibility layer; faking UID 0 does not grant kernel privileges.
4. `isolatedProcess` has no permissions, no sockets, no direct filesystem access, and cannot execute app data — useful only for narrow parsing.
5. Per-process network filtering has no stable public API; our seccomp-broker approach is conditional on device validation. `VpnService` is UID-scoped and requires user consent.
6. W^X prevents `execve` of project files; scripts are run through an interpreter from a permitted location.
7. Android 12+ phantom-process controls and vendor battery managers can terminate sessions; foreground service, notification and wake lock mitigate but cannot guarantee.
8. Background execution and foreground-service start restrictions apply (target SDK 36).
9. Keystore protects keys, not the app process itself; a compromise of the main process is a compromise of broker functionality.
10. Passing tests reduce risk; they are not proof of complete security.

---

## 11. Multiple terminal session architecture

```
TerminalRegistry (Rust)
 ├─ Session { id, name, cwd, env, shell_history, pty_master_fd,
 │            pgid, child_pid, scrollback, cols/rows, network_mode,
 │            policy_profile, state, created_at, last_used }
 ├─ Session { ... }
 └─ Supervisor
      ├─ spawn(config) -> validates cwd under project, scrubs env, installs limits
      ├─ on_output(fd) -> vte -> diff -> JNI event
      ├─ resize(id, cols, rows) -> TIOCSWINSZ
      ├─ signal(id, sig) / interrupt(id) / terminate(id)
      ├─ reap() -> waitpid(WNOHANG), update state, notify UI
      └─ stop_all() -> SIGTERM group, 5 s grace, SIGKILL group
```

- UI: session chips `[Python] [Rust] [Agent] [+]`, rename via long-press, close button, working-directory display, per-session Stop button, and a global Stop All.
- Independent state per session: process group, PTY, cwd, environment, history, scrollback, network mode, policy profile.
- Session cap: 6 interactive sessions on phones, 12 on tablets (phantom-process budget). Exceeding the cap requires closing another session.
- Persistence: SQLite stores names, cwd, env overrides, scrollback size and last-active session. **Live processes cannot be restored after the OS kills the app**; restoration recreates a shell in the same cwd and displays an explicit "previous process ended" notice.
- Abandoned processes are reaped by the supervisor; the foreground service notification shows session count and offers End all.

---

## 12. AI agent compatibility

Priority order from the brief:

1. **OpenCode**: primary path is an OpenCode-compatible *client/session model* in ForgeBox using the local tool gateway and a user-configured endpoint. Running the desktop CLI locally is attempted only inside a PRoot guest with a working Node/Bun runtime and is labelled best-effort.
2. **OpenAI Codex CLI**: same two paths (guest userspace best-effort, remote SSH reliable).
3. **Other compatible agents**: generic agent runner that speaks our tool-gateway JSON-RPC.
4. **OpenAI-compatible API providers**: direct HTTP from the Kotlin network broker with Keystore-stored keys; no keys in agent env.
5. **Ollama-compatible endpoints**: HTTP client to user-configured endpoint (remote or LAN). Local on-device inference via `llama.cpp` is a later research item, not an M10 promise.

Agent session features: project binding, output streaming with tool-call cards, approval requests, cancellation token, file-change tracking (hash before/after), mandatory checkpoint, reviewable diff, restore/discard. API keys are redacted in output and logs; keys are never included in prompts constructed by ForgeBox itself.

---

## 13. GitHub authentication

- Primary: **OAuth 2.0 Device Authorization Grant (RFC 8628)** with the user opening `github.com/login/device` and entering a code; no client secret in the app.
- Secondary: fine-grained Personal Access Token entered through a secure Compose field; scoped to the selected repositories with minimum permissions.
- Enterprise option: GitHub App installation tokens generated by an operator-controlled service.
- Tokens are encrypted with an Android Keystore-backed AES-GCM key (via `androidx.security:security-crypto` or a small JNI bridge) and never written to project files, `.git/config`, environment variables, or crash reports.
- `git2` credential callback asks the Kotlin broker for a short-lived token over a local, app-private channel; the token is supplied only for the duration of one remote operation and is not visible to terminal children.
- Local Git works with no network; remote operations are explicit user actions and are shown in the network monitor.

---

## 14. Network architecture

Modes:

| Mode | Behaviour |
|---|---|
| Local-only | All broker requests denied; local editing, terminal, Git and offline builds work |
| Controlled | Allowlist of GitHub, configured AI providers, package repositories, documentation hosts, configured Ollama/SSH endpoints |
| Temporary broader | Time-boxed (default 15 min, max 60), per project or per session, persistent banner and countdown |

Design:

- All app-initiated HTTP(S) goes through Kotlin `OkHttp` broker. Rust `net_policy` returns `ALLOW` / `DENY` / `ASK` and records destination, process/agent id, bytes up/down, and cancellation handle.
- `ConnectivityManager` + `Network.bindSocket` bind allowed sockets to the selected network; offline detection is immediate.
- Child-process network control: seccomp-based denial of `socket`/`connect` plus a local HTTP/CONNECT broker (R-04). If validation fails on target devices, we do **not** claim per-process isolation; we present a per-session network toggle with an explicit warning.
- `VpnService` was evaluated and rejected for per-process filtering in v1: it is UID/package scoped, requires user consent, and cannot distinguish two child processes of ForgeBox.
- Before uploading project source to a new provider/destination: modal sheet with destination hostname, provider, approximate file/byte count, and explicit Allow / Deny. No silent uploads.
- Network activity UI: live destination list, requesting process/agent, up/down counters, download progress, Cancel buttons.

---

## 15. Storage structure and protection

See §7.1. Additional rules:

- `Projects/` is the only tree exposed to shell/agent tools as "the workspace".
- `Imports/` is a quarantine: archives are inspected there and extracted into a fresh directory only after validation.
- `Exports/` holds SAF-authored output; SAF URIs are persisted with persistable permissions where supported.
- `internal/db`, `internal/audit`, `internal/checkpoints`, `internal/journal` are never listed as projects and are not reachable through agent tool calls.
- `internal/secrets` contains only Keystore-wrapped ciphertext. Plaintext tokens exist only in broker memory.
- SQLite schema: projects, files_index, tabs, sessions, terminal_history, checkpoints, audit_index, network_log, build_history, config. WAL mode, foreign keys, and indexed FTS5 for search.
- Backups of project data are checksummed; restores are verified and atomic (staging dir + rename).

---

## 16. Dependencies

### 16.1 Rust crates (built for `aarch64-linux-android`, `cargo-ndk`)

| Area | Crate | Notes |
|---|---|---|
| JNI | `jni` | JNI façade |
| Serialization | `prost`, `prost-types` | protobuf wire format |
| Async | `tokio` | Android-compatible |
| Text | `ropey`, `unicode-segmentation` | document mirror |
| Parsing | `tree-sitter` + grammars | incremental parsing/index; grammar cross-compilation needed |
| Search | `regex`, `ignore`, `walkdir`, `similar` | find/replace, diff |
| Terminal | `vte`, `libc`/`nix` (direct Bionic PTY FFI) | `portable-pty` Android support not assumed |
| Git | `git2` + `vendored-libgit2`, `openssl-sys` vendored (HTTPS first; SSH later) | libgit2 1.9+ vendored; SSH via libssh2 is a later milestone |
| Storage | `rusqlite` + `libsqlite3-sys` bundled | FTS5 |
| Archives | `zip`, `flate2`, `tar`, `xz2`, `zstd` | validation-first extraction |
| Security | `seccomp` (spike), `nix::sys::resource`, `sha2`, `hmac` | filters, rlimits, hashes |
| Filesystem | `inotify` (with stat-scan fallback), `tempfile` | change detection |
| Observability | `tracing`, `tracing-subscriber` | redacted logs only |

### 16.2 Kotlin / Android

- Jetpack Compose, Material 3, `WindowSizeClass`, `AndroidView`, `DocumentFile`/SAF.
- `androidx.security:security-crypto`, `androidx.biometric`, `androidx.work` (where appropriate).
- OkHttp, `ConnectivityManager`, `Network.bindSocket`.
- sora-editor `editor`, `language-treesitter`, (optional `editor-lsp`, marked experimental upstream), LGPL-2.1 — attribution and dynamic linking obligations must be reviewed.
- Optional: `:proot-engine` AAR from `oonid/pr` (GPL-2.0 proot, GPL-3.0 proot-distro plugins) — licence review required before distribution.
- Build: Android Studio / AGP 9.x, Gradle 9.x, JDK 17+, NDK r27c+, Kotlin 2.x, `cargo-ndk` 4.x, `minSdk 31`, `targetSdk 36`, `compileSdk 36`, ARM64 primary with x86_64 for emulator.

### 16.3 Licensing gate

Before M14, an explicit licensing review must cover: sora-editor (LGPL-2.1), tree-sitter grammars (mostly MIT), proot (GPL-2.0), proot-distro plugins (GPL-3.0), OpenSSL (Apache-2.0), libgit2 (GPL-2.0 with linking exception), CPython (PSF), toybox/mksh ( BSD/ISC-like), and any packaged binaries. No GPL binary is shipped until source-offer obligations are satisfied.

---

## 17. Development milestones

| # | Milestone | Concrete deliverable | Exit criteria |
|---|---|---|---|
| 1 | Architecture / threat model | This document + risk register + decisions | Approved by project owner |
| 2 | Android foundation & Compose shell | Empty app, navigation, responsive layouts, FGS skeleton, JNI hello | Builds and runs on API 31 and 36; no permissions beyond needed |
| 3 | Rust core & JNI | `forgebox_core` with protobuf façade, workspace types, SQLite, event sink | Host tests green; JNI round-trip on device |
| 4 | Private workspace & explorer | Project CRUD, SAF import/export, validated vfs, audit | Traversal/symlink tests pass |
| 5 | Professional editor | sora-editor host, tabs, Rust mirror, search, toolbar, recovery | Feature checklist from §4.3 demonstrated |
| 6 | Terminal & PTY | Real `/system/bin/sh` over PTY, vte renderer, scrollback, resize | Runs real commands; no fake output |
| 7 | Multiple sessions | Registry, tabs, rename/close, Stop All, FGS persistence | Independent sessions verified; no cross-session kills |
| 8 | Security & supervision | Policy engine, confirmations, seccomp spike, limits, checkpoints, audit | Security suite green; isolation limits documented |
| 9 | Git & source control | `git2` status/diff/stage/commit/history + UI + auth broker | Local flow works offline; remote flow with device flow |
| 10 | Agent compatibility | Tool gateway, checkpoint-before-edit, diff review, remote SSH agent | Agent edit can be reviewed and restored; cancel works |
| 11 | Networking & providers | Three modes, broker, activity UI, upload consent | Local-only works offline; no silent transfers |
| 12 | Build & test environment | Python/Rust/Node/C/C++ via runtime packs, PRoot or SSH; diagnostics | Real builds for at least Python and one compiled language |
| 13 | Checkpoints/recovery/backups | Full snapshot/diff/restore, crash replay, project backups | Restore after agent damage verified |
| 14 | Audit, performance, release | Security audit, perf tuning, licence review, signed release | Critical failures = release blocker; docs state limitations |

For each milestone: implement → test → fix failures → security review → document limitations → commit working changes. Placeholders are never marked complete.

---

## 18. Complexity estimate

Full scope (M1–M14): **approximately 22–35 person-months** for a small experienced team, excluding hardware/OS certification.

| Workstream | Estimate |
|---|---|
| M2–M4 foundation, Rust core, workspace/explorer | 4–6 pm |
| M5 editor integration + tooling | 3–5 pm |
| M6–M7 terminal, PTY, sessions | 4–6 pm |
| M8 security, policy, supervision, audit | 4–6 pm |
| M9 Git + auth | 2–3 pm |
| M10–M11 agents + networking | 4–6 pm |
| M12 build environments + PRoot/SSH integration | 3–6 pm |
| M13 checkpoints/recovery | 2–3 pm |
| M14 audit, perf, packaging, docs | 3–5 pm |

High-variance items: PRoot compatibility across vendor ROMs, seccomp validation, LSP quality, and maintaining tree-sitter grammars. These dominate the upper bound.

---

## 19. Risk register

| ID | Risk | Sev | Likelihood | Mitigation / kill criterion |
|---|---|---|---|---|
| R-01 | Perceived strong isolation that does not exist | Critical | High if unmanaged | Product docs and UI state same-UID limitation; never market PRoot as sandbox; security review gate |
| R-02 | sora-editor / android-tree-sitter maintenance gaps (upstream bindings unmaintained) | High | Medium | Pin versions, vendor/fork critical bindings, budget maintenance, fallback to TextMate grammar path |
| R-03 | W^X prevents intended execution path | Critical | High if naive | Use `/system/bin/sh`, nativeLibraryDir binaries, PRoot loader; test on targetSdk 36 device early |
| R-04 | Additional seccomp filter / socket denial rejected or unstable on vendor ROMs | High | Medium | Spike at M8 on ≥3 devices; if unavailable, degrade to broker-only with explicit warning and no per-process claim |
| R-05 | PRoot breaks on a vendor ROM / Android update | High | Medium | Optional backend, version-pinned engine, SSH fallback, diagnostics screen |
| R-06 | Phantom-process killing of sessions | High | High | Cap sessions, FGS + notification + wake lock, document developer option, never promise guaranteed background survival |
| R-07 | Credential leakage into terminal/logs/commits | Critical | Medium | Keystore-only, broker-only plaintext, output redaction, no env/args, redaction tests, git config checks |
| R-08 | Dual text ownership (widget vs Rust) corrupts buffers | High | Medium | Clear authority rule (§4.2), journal replay tests, conflict UI, fuzz edit sequences |
| R-09 | Archive extraction escape / bomb | Critical | Medium | Two-pass validation, no symlinks/devices, size/inflation/depth limits, quarantine, proptest fuzzing |
| R-10 | GPL/LGPL obligations block distribution | High | Medium | Licence gate at M2 and M14, prefer dynamic linking/attribution, legal review for PRoot |
| R-11 | Full 18-language LSP support unattainable | Medium | High | Scope LSP to "where supported"; tree-sitter symbols always available |
| R-12 | Scope exceeds schedule | High | High | Strict milestone gates; no placeholder marked complete; defer heavy build environments to M12 and prefer SSH |

---

## 20. Resolved decisions (2026-11-22)

| Decision | Outcome | Consequence |
|---|---|---|
| Distribution target | **Google Play** | `minSdk 31`, `targetSdk 36`, `compileSdk 36`; Play policy and licence obligations enforced from M2 |
| Licence posture | **LGPL-2.1 / GPL-2.0 accepted, with review** | sora-editor may be used; optional `:proot-engine` AAR may be integrated; attribution and source-offer obligations are a hard M14 release gate |
| Execution fallback | **SSH remote backend accepted** | Toolchains and agent CLIs that cannot run on-device are offered as explicit remote sessions with local UI; never presented as local execution |
| Milestone order | **Strict M2 → M14** | Implementation follows the specified sequence; no skipping ahead to terminal/agents before foundation, Rust core and explorer are complete |

Still open (does not block M2): the physical device/ROM fleet for the R-03/R-04/R-05 spikes must be confirmed before M8.
