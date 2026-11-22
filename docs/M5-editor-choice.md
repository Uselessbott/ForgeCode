# M5 — Editor Component Evaluation

## Context

M5 needs a professional code editor for ForgeBox (Android 12+, ARM64 primary, Compose UI, Rust core owns file I/O). `PROJECT FORGEBOX.txt` was not present in the workspace at evaluation time; requirements were taken from the user M5 brief and `docs/ARCHITECTURE.md` (§4, §6 references). The architecture already recommended sora-editor.

## Candidates

### A. sora-editor (`io.github.Rosemoe.sora-editor`)

- **License**: LGPL-2.1. Compatible with ForgeBox's stated "LGPL-2.1/GPL-2.0 accepted, with review" posture; source/attribution/relink obligations must be tracked for release.
- **Maturity/maintenance**: Active GitHub project (Rosemoe/sora-editor), 2.7k+ commits, releases through 0.24.x. 0.24.6 raised min behavior changes; 0.24.7 (pre-release) raises minSdk to 23 and changes diagnostics/inlay/highlight APIs.
- **Min SDK**: 23 in 0.24.7; earlier 0.24.x supported lower. ForgeBox minSdk is 31, so compatible.
- **Compose interop**: Android `View` (`CodeEditor` extends View) hosted via Compose `AndroidView`. Mature pattern; no official pure-Compose implementation.
- **Incremental parsing/highlighting**: Yes. Core supports incremental analysis; `language-textmate` provides TextMate grammar tokenization; `language-treesitter` provides Tree-sitter incremental parsing/highlight/folding/brackets/locals when grammars/scm queries are supplied.
- **Language coverage for spec list**: TextMate grammars exist for most requested languages (Java/Kotlin/Python/JS/TS/C/C++/Rust/Go/HTML/CSS/JSON/XML/YAML/Markdown/Bash/SQL). Packaging quality varies; grammar bundles must be vetted. Tree-sitter grammars can cover same set but require NDK-built native libs and scm query assets.
- **Multi-cursor**: Not prominently documented as a first-class stable feature in README; selection APIs exist but full mobile multi-cursor UX is uncertain. **Defer unless verified during implementation; do not fake.**
- **Folding/brackets/auto-indent**: Supported by core/language modules (TextMate language-config provides brackets/indent patterns; Tree-sitter supports folding/brackets queries).
- **Find/replace/go-to-line**: Built-in search/replace and go-to-line/jump features available in editor.
- **Physical keyboard/IME**: Designed for Android touch + keyboard; key event handling exists. Hardware keyboard shortcuts need explicit verification/mapping.
- **Known gaps**: LSP module is experimental/upstream-moving; some APIs changed in 0.24.7; TextMate regex engine behavior depends on oniguruma/native support; Tree-sitter grammar packaging/16KB page-size/native ABI maintenance is non-trivial; large-file behavior depends on configuration.
- **Security notes**: Native/editor library, no WebView, no remote script execution. Grammar/theme files are local assets; do not load remote grammars.

### B. Other Android code editor libraries (CodeView/CodeEditor/ACE/web wrappers)

- **Examples**: CodeView, Android-code-editor forks, AceEditor WebView wrappers.
- **License**: Varies; many are MIT/Apache but maintenance is inconsistent.
- **Maturity**: Generally lower than sora-editor for modern editing features. Many are syntax-highlighting viewers, not full editors.
- **Compose interop**: Usually View/WebView; WebView wrappers add JS bridge complexity.
- **Incremental parsing/multi-cursor/folding**: Often incomplete or absent.
- **Verdict**: Rejected for M5. Higher integration risk, weaker feature coverage, or WebView trust/IME surface.

### C. Custom Compose editor + Rust rope/document model

- **License**: Fully controlled.
- **Maturity**: New implementation; very high effort.
- **Compose interop**: Native.
- **Incremental parsing**: Possible with tree-sitter in Rust, but rendering/IME/selection/magnifier/accessibility/keyboard/multi-cursor/folding are large projects.
- **Verdict**: Rejected for M5/v1. Would delay core product and likely produce inferior touch editing. Revisit only if sora-editor becomes unmaintainable.

## Decision

**Choose A: sora-editor**, using **`editor` + `language-textmate`** for M5.

### Why

1. Matches architecture decision and M5 schedule.
2. Provides real editor capabilities (not TextField): line numbers, undo/redo, search/replace, syntax highlighting, brackets/indent/folding hooks, text selection, IME handling.
3. Compose interop is straightforward through `AndroidView`.
4. LGPL-2.1 is acceptable under ForgeBox licensing posture with release review.

### M5 scope choice

- Use **TextMate** first, not Tree-sitter, for M5: faster to integrate, no extra native grammar .so packaging, adequate for syntax highlighting/indent/brackets/language config across requested languages.
- Tree-sitter can be added later for better incremental parsing/indexing/symbol features where needed.
- Do **not** add LSP in M5 (`editor-lsp` is experimental and M5 rule forbids LSP).

## Version / dependency plan

- Pin sora-editor to a Maven Central release available during build; prefer latest stable 0.24.x verified in Gradle. If 0.24.7 API breaks compile, fall back to 0.24.6.
- Modules: `io.github.Rosemoe.sora-editor:editor`, `io.github.Rosemoe.sora-editor:language-textmate`.
- Min SDK remains 31 (app), above sora-editor requirement.

## Feature disposition (M5)

| Spec §6 feature | M5 plan | Notes |
|---|---|---|
| Syntax highlighting | Implement | TextMate grammars/assets for common languages; graceful fallback to plain text. |
| Line numbers | Implement | sora-editor core. |
| Folding | Implement if provided by selected language mode; otherwise defer | TextMate folding depends on language config/grammar support. |
| Auto-indent | Implement | Language config/indent rules. |
| Bracket match/completion | Implement where supported | Component/language config. |
| Multiple tabs | Implement | Compose tab bar + ViewModel tab state. |
| Split editor | Defer M5.x/M6? | Explicitly deferred; no fake split. |
| Multiple cursors | Defer unless natively solid | Do not bolt on half-working implementation. |
| Find/replace in file | Implement | Use editor built-in search/replace. |
| Find in files | Implement simple O(files) scan | Rust ListFiles+ReadFile via repository; not indexed. |
| Regex/case/whole word | Implement where supported | In-file via editor search options if exposed; find-in-files simple regex in Kotlin/Rust-safe read path. |
| Go to line | Implement | Simple dialog/command. |
| Go to symbol/definition/rename | Defer | Needs indexer/LSP. |
| Formatting | Defer | Needs toolchain/formatter integration/policy; no fake format. |
| Autosave | Defer (or simple only if conflict-safe) | Default defer; manual save in M5. |
| Crash recovery/tab restore | Defer M13 | Checkpoint/journal integration later. |
| Large files | Implement guard | Size threshold; refuse full editor load above cap. |
| UTF-8, LF/CRLF | Implement | Detect on load, preserve on save; reject/readonly for invalid UTF-8. |
| Mobile programming toolbar | Implement | Scrollable extra keys wired to editor key/command API. |
| Bluetooth/physical keyboard | Implement basic support | Verify key events; document gaps. |

## Risks

- Grammar asset size/licensing for 18 languages.
- TextMate bundle format/regex compatibility.
- 16KB page size/native lib concerns if Tree-sitter added later.
- API drift between sora-editor releases; pin version.
