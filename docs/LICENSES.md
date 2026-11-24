# LICENSES / Third-Party Notices (M11 seed)

This file is a **seed** for M14 license aggregation. It records known M10.1/M11 third-party artifacts and obligations. It is **not** a complete inventory.

## Status

- Option B: no emulator runtime verification on a supported device; only API 28 emulator available (below minSdk 31).
- Full dependency/license scan deferred to M14.

## Bundled / Direct Runtime Artifacts

| Component | Version | License | Source / Upstream | Notes / Obligation |
|---|---:|---|---|---|
| Alpine minirootfs (aarch64) | 3.20.7 | Mixed (Alpine packages; various licenses) | https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/aarch64/alpine-minirootfs-3.20.7-aarch64.tar.gz | SHA-256: `559ca287bba1b4c9640d57bcb8800afc562c5ded70823f2e98d67ec03bfedf0b`. Record per-package license via Alpine package DB. **Verify before release** that bundled minirootfs contains no GPL binary requiring source-offer handling beyond what is documented; if any GPL package is present, record corresponding source availability. |
| Alpine minirootfs (x86_64) | 3.20.7 | Mixed (Alpine packages; various licenses) | https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/alpine-minirootfs-3.20.7-x86_64.tar.gz | SHA-256: `1a6e5b959061b0aebff5eb79c569cee206636c59e3d443f59456ada850fa9255`. Same verification note as aarch64. |
| PRoot | 5.1.107.102 (Termux) | GPL-2.0 | https://github.com/termux/proot (upstream); Termux package: https://packages.termux.dev/apt/termux-main/pool/main/p/proot/ | Bundled as `libproot.so`, `libprootloader.so`, `libprootloader32.so` in `app/src/main/jniLibs/*`. **Source-offer plan required at release**: provide corresponding source or written offer per GPL-2.0. Record exact Termux build/source revision used for release. |
| libgit2 / git2-rs | workspace-pinned via Cargo.lock | libgit2: GPL-2.0 WITH Linking Exception (verify exact upstream text) | https://libgit2.org/ , https://github.com/rust-lang/git2-rs | Do not claim license without verifying bundled/Cargo.lock version text. M14 must capture exact versions and license texts. |
| OpenSSL | Android/system or bundled as applicable | Apache-2.0 (OpenSSL 3.x) / verify actual linked version | https://www.openssl.org/ | Verify actual runtime/NDK linkage before release; record exact version and notice. |
| OkHttp | 4.12.0 | Apache-2.0 | https://square.github.io/okhttp/ | Used by M11 network broker and M9 device-flow scaffold. |
| AndroidX / Jetpack Compose / Material3 | BOM-pinned | Apache-2.0 | https://developer.android.com/jetpack | See Gradle dependency/BOM versions for exact artifacts. |
| sora-editor | as integrated | LGPL-2.1 | https://github.com/Rosemoe/sora-editor | Dynamic linking/usage must preserve attribution and LGPL obligations; verify integration mode before release. |

## Policy Notes

### Rust crates
- **TODO M14**: run `cargo-license` or `cargo-deny` in `rust/forgebox_core` and record full crate inventory.
- Any GPL/LGPL/AGPL crate is a **blocker** unless its license obligations are satisfied and source-offer/attribution is recorded.

### Kotlin/Java dependencies
- **TODO M14**: run a Gradle license scanner (e.g. `com.github.jk1.dependency-license-report` or equivalent) and record full dependency inventory.
- Verify transitive licenses for OkHttp, AndroidX, Compose, test libs.

### Release checklist (M14)
- [ ] Enumerate Rust crate licenses from Cargo.lock.
- [ ] Enumerate Gradle dependency licenses.
- [ ] Verify Alpine minirootfs package licenses / GPL content.
- [ ] Provide PRoot GPL-2.0 source offer/corresponding source.
- [ ] Verify libgit2 exact license text/version.
- [ ] Verify OpenSSL version/license notice.
- [ ] Include third-party notices in app/release artifact as required.
