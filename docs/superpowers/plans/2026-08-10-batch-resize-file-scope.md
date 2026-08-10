# Batch Resize File Scope Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add IMS-only, non-IMS-only, and all-file scopes to batch resize, including safe ordinary-image JPEG replacement and visible unsuccessful-file reporting.

**Architecture:** Keep the existing IMS transformer unchanged and add a focused ordinary-image transformer in `ims_pack.rs`. Let `start_batch_resize` scan and dispatch a typed file task, normalize both transformer outcomes into one batch summary, and emit the summary through the existing progress event. Extend the existing modal without introducing a second batch command.

**Tech Stack:** Tauri 2, Rust 2021, `image`, `avif-rs`, Rayon, vanilla JavaScript, HTML/CSS, Vite.

---

### Task 1: Ordinary Image Transformer

**Files:**
- Modify: `src-tauri/src/ims_pack.rs`
- Test: `src-tauri/src/ims_pack.rs`

- [ ] Add failing tests that create temporary JPEG/PNG files and assert: oversized files shrink, undersized files remain byte-identical, PNG-to-JPEG removes the PNG only after creating the JPEG, and an existing JPEG target produces a conflict without changing either file.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml standard_image -- --nocapture` and verify the new tests fail because the ordinary-image API is absent.
- [ ] Add `StandardResizeOutcome` and `resize_standard_image(path, max_edge, convert_to_jpeg, enable_resize)`. Decode AVIF through the existing AVIF helper, encode the selected output format, write a same-directory temporary file, and only then replace the source or create the JPEG target.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml standard_image -- --nocapture` and verify all ordinary-image tests pass.

### Task 2: Scope-Aware Scanning and Dispatch

**Files:**
- Modify: `src-tauri/src/ims_pack.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/ims_pack.rs`

- [ ] Add scanner tests with IMS names, ordinary supported images, and unsupported files; assert `ims_only`, `standard_only`, and `all` return the exact expected paths without classifying `.ims.png` as ordinary PNG.
- [ ] Run the scanner tests and verify they fail before adding the scope API.
- [ ] Add a serde-compatible `ResizeFileScope` with default `ims_only`, a typed resize task, and a unified scanner that respects folder depth and de-duplicates paths.
- [ ] Update serial and Rayon branches in `start_batch_resize` to dispatch IMS and ordinary files while preserving IMS cache invalidation only for modified IMS files.
- [ ] Add a resize-specific progress payload carrying success, skipped, failed, and issue details on completion; record target conflicts as skipped issues and processing errors as failed issues.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml` and verify the Rust suite passes.

### Task 3: Scope and Result UI

**Files:**
- Modify: `index.html`
- Modify: `src/app.js`
- Modify: `src/styles.css`

- [ ] Add a `resize-file-scope` select with values `ims_only`, `standard_only`, and `all`; add an initially hidden result summary, unsuccessful-file list, and copy button.
- [ ] Include `file_scope` in `collectResizeConfig`, reset it to `ims_only` when opening the modal, and change scan text based on the selected scope.
- [ ] Render completion counts and issue rows from the progress payload, keep the detailed list hidden when empty, and copy tab-separated status/path/reason text through the Clipboard API.
- [ ] Add focused styles that keep a long issue list scrollable without expanding the modal beyond the viewport.
- [ ] Run `npm run build` and verify Vite completes successfully.

### Task 4: Final Regression Verification

**Files:**
- Verify: `src-tauri/src/ims_pack.rs`
- Verify: `src-tauri/src/lib.rs`
- Verify: `src/app.js`
- Verify: `index.html`
- Verify: `src/styles.css`

- [ ] Run `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`; if it reports formatting changes, run `cargo fmt --manifest-path src-tauri/Cargo.toml` and repeat the check.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml` and verify all tests pass.
- [ ] Run `npm run build` and verify the production bundle builds.
- [ ] Inspect `git diff --check` and `git diff` to confirm no whitespace errors, unrelated edits, placeholder text, or inconsistent field names remain.
