# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Muffin Tidy（媒体文件整理工具）—— a Windows-first Tauri 2 desktop app for **triaging/culling photo & video libraries**. Flow: pick a directory → recursive scan classifies media and pairs Live Photos → user reviews a virtualized grid and marks/removes files via keyboard → (an export/apply step is not yet implemented; see "Triage has no sink").

Stack: Tauri 2 (Rust backend) + Vue 3 `<script setup>` + Pinia + Tailwind v4 + TypeScript + Vite. Comments and UI strings are in Chinese — match that when editing.

## Commands

`tauri.conf.json` hardcodes `bun run dev` / `bun run build` as the before-dev/before-build hooks, so the Tauri CLI path needs **bun** on PATH (a `pnpm-lock.yaml` also exists, but the Tauri lifecycle calls bun). Run the Tauri CLI itself with whichever PM you prefer:

- **Dev (full app):** `bun run tauri dev` — Tauri launches the vite dev server on port **1420** (strict) and opens the native window with hot reload for both sides.
- **Frontend only (browser):** `bun run dev` — vite on 1420, no Rust. Most backend features (scan/thumbnail/WIC) won't work without Tauri.
- **Production build:** `bun run tauri build` — bundles installers for all targets.
- **Typecheck:** `bun run build` runs `vue-tsc --noEmit && vite build` (no standalone lint/typecheck script — this is the gate).
- **Rust-only check:** `cargo check` / `cargo build` inside `src-tauri/`.

There is **no test framework** wired up (no test script, no test files). Don't assume `bun test` / `cargo test` do anything meaningful yet.

## Architecture

Two processes talking over Tauri IPC: a Rust backend (`src-tauri/src/`) exposing `#[tauri::command]`s, and a Vue SPA (`src/`) calling them via `invoke`. There is **no vue-router** — `App.vue` swaps views off a phase enum in the single Pinia store.

### Backend (`src-tauri/src/`)

- **`lib.rs`** — `run()` builds the Tauri app: registers plugins (opener, dialog, fs), the `mtidy-mphoto` URI scheme, and the command handlers `scan_directory`, `preload_thumbnails`, `get_thumbnail`, `get_preview`. Thumbnails/previews are returned to JS as base64 `data:` URLs.
- **`scanner.rs`** — `scan()` walks the dir (`walkdir`, skips dotfiles), classifies by extension into `Img`/`Vdo`/`Lpo`, reads capture time + a small EXIF subset. The hard part is **`pair_live_photos`**: it fuses an image + its motion video into one `Lpo` entry and *removes the standalone video entry from the list*. Two passes: (1) iOS `ContentIdentifier` (EXIF tag `0x9999` / `apple-fi`); (2) same-dir same-stem fallback where `.mov`→Apple, `.mp4`→Huawei.
- **`livephoto.rs`** — raw XMP/EXIF byte parsing (no full decode). Detects Android Motion Photo flags, reads the iOS content identifier, and computes `MicroVideoOffset` (the byte offset *from EOF* where the MP4 is embedded inside the JPEG).
- **`thumb.rs`** — thumbnail (320px) / preview (2048px) generation with a **multi-tier decode fallback** and a disk cache in `%TEMP%/mtidy-thumbs` keyed by `path + mtime` (source edits auto-invalidate). Decode order: video → Windows Shell thumbnail; JPEG → DCT-downscaled fast path (`jpeg-decoder`, avoids decoding huge images to full res); common formats → `image` crate; HEIC/RAW → Windows WIC; all-else → gray placeholder (never a broken-icon).

**Windows-specific decoding is load-bearing.** HEIC/RAW (WIC `IWICImagingFactory`) and video first-frames (Shell `IShellItemImageFactory`) both use COM/MTA and are `#[cfg(windows)]`. On non-Windows these formats fall straight through to the gray placeholder — there is no portable path. The `windows` crate features needed are pinned in `Cargo.toml` under `[target.'cfg(windows)'.dependencies]`.

### `mtidy-mphoto://` custom URI scheme

Android Motion Photos store the MP4 *appended inside the JPEG*. `mphoto_protocol` (in `lib.rs`) reads `MicroVideoOffset`, seeks to that offset, and streams the trailing bytes as `video/mp4`. The frontend builds `mtidy-mphoto://video?path=<urlencoded>` for `liveType === "android"`. iOS/Huawei live photos instead point at a *separate* video file via `convertFileSrc`. This split lives in `stores/project.ts` when building `videoUrl`.

### Frontend (`src/`)

- **`stores/project.ts`** — one Pinia store = all app state (files, `focusedIndex`, phase, undo history, viewer flag). The **phase state machine** `idle → scanning → preloading → ready` drives view switching in `App.vue`. `openDirectory()` invokes the scan, maps snake_case `ScanResult` → camelCase `ProjectFile`, then `startPreload()` warms the thumbnail cache (Rust-side, 2 threads, progress via `Channel`; user-skippable).
- **Triage model:** every file has status `normal | marked | removed`. `removed` means *removed from the working set, NOT deleted from disk*; `visibleFiles` filters it out. `history` stacks status changes for Ctrl+Z undo.
- **`components/ThumbnailGrid.vue`** — virtualized via **`virtua`** `VList`, grouped into rows by computed column count. Ctrl/⌘+wheel = continuous zoom (adjusts persisted `thumbSize`, which redefines columns).
- **`composables/useThumbQueue.ts`** — global semaphore (max 4 concurrent) around `get_thumbnail` invokes so scrolling doesn't flood the backend.
- **`composables/useKeyboard.ts`** — global shortcuts (↑↓←→ navigate, Space mark, D/Del remove, Ctrl+Z undo). Suspended while the Lightbox is open — **`Lightbox.vue` owns its own keydown handler** so the two never double-fire.
- **Src resolution rule (repeated in `PreviewPanel` & `Lightbox`):** natively-decodable formats use `convertFileSrc(path)` directly; WIC formats and videos must round-trip through Rust (`get_preview`/`get_thumbnail` → base64). `needsRustConvert()` + `WIC_EXTS` in the store is the single source of truth for which path a file takes.

### Frontend ↔ backend data contract

Rust `ScannedFile` is serialized `snake_case`; the store maps it to a `camelCase` `ProjectFile`. Enums must stay in sync on both sides: `MediaType` = `img|vdo|lpo`, `LiveType` = `apple|android|huawei`. Changing either enum means editing both `scanner.rs` and `stores/project.ts`.

### Export pipeline (`export.rs`)

The triage sink is `export_files` (in `export.rs`): it **copies** the kept files to a user-picked target dir, organized as `target/YYYY/MM/{Img|Vdo|Lpo}-{YYYYMMDDHHMMSS}-{seq}.ext` (toggle off → flat copy with original names). Driven by `ExportDialog.vue` (opened from the StatusBar button or `E`), with a progress `Channel` and a summary.

**Non-negotiable safety invariants (enforced in Rust, covered by unit tests):**
- Never write to the source tree: `validate_target` rejects target == source, target inside source, or source inside target.
- Never overwrite / never touch source: copy-only via `std::fs::copy`; `copy_preserving` refuses if dst exists or resolves to the source file; sequence numbers find a free name.
- Preserve everything: byte-exact copy keeps all file headers (EXIF/XMP/container); `filetime` additionally restores mtime/atime.

Live Photos export as a pair sharing a base name; Android motion photos (`video_path == source_path`, MP4 embedded in the JPEG) copy the single file. SHA-256 powers both batch dedup and target-conflict skipping. The copy runs entirely in Rust, so **no `fs` write capability is needed** — the read-only `fs` scopes in `capabilities/default.json` are unrelated to export. (Picking the target uses `dialog:allow-open`; revealing the folder/log uses `opener:allow-open-path`.)

Every real (non-dry-run) export writes an **audit manifest** `muffin-tidy-export-{timestamp}.log` into the target root: header (source/target/options) + one line per file (`[导出] src -> dst`, `[重复]`, `[冲突]`, `[失败]`) + summary. The path comes back in `ExportSummary.log_path` and the dialog links to it. `.log` isn't a media extension, so re-scanning the target won't pick it up.

## Conventions

- **Custom titlebar:** window runs with `decorations: false`; `TitleBar.vue` provides the drag region (`data-tauri-drag-region`) and min/max/close controls. Window permissions are enumerated in `capabilities/default.json` — new window ops need a matching `core:window:*` permission there.
- **Theming:** dark-only, all colors are CSS variables in `src/style.css` (`--bg-base`, `--accent`, `--marked`, …). Use the variables, don't hardcode hex.
- **Frontend naming (from global rules):** folders `kebab-case`, Vue components `PascalCase.vue`, composables/utils `camelCase.ts`.
- `src-tauri/target/` is build output — ignore it when searching.
