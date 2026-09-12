# Red Strap architecture

Three crates, one data directory, zero placeholders. This note explains how
the pieces fit together so a new contributor can find anything in a minute.

## Crates

```text
crates/core/        redstrap-core   shared library (no UI, no CLI)
crates/bootstrap/   RedStrap        launcher CLI: install, deploy, launch, watch
crates/desktop/     RedStrap-Settings  native settings app (egui)
```

`core` owns all behaviour; `bootstrap` and `desktop` are thin front-ends.
Anything both need (settings, pipeline, updater, Discord IPC, Roblox log
parsing) lives in `core` so it is implemented exactly once.

## Data layout (`%LocalAppData%\RedStrap`)

Defined by `core::paths::Layout`: `Settings.json`, `State.json`,
`ClientAppSettings.json` (generated flags), `Versions/`, `Downloads/`,
`Modifications/`, `Logs/`, `Cache/`. All loaders are defensive — a corrupt
file is renamed to `.corrupt-<timestamp>` and replaced with defaults.

## Launch flow (`bootstrap`)

1. Parse CLI (`main.rs`), acquire layout, load settings/state, init logging.
2. Optional: install/repair (`install.rs`), background self-update (`updater`
   via `background_update`), protocol-URL handling.
3. `pipeline::ensure_installed` — resolve channel → version GUID → parallel
   download (resume + MD5 verification per the Roblox manifests) → extract
   → apply flags/mods/cursors/recolor → write distribution state.
4. `launch::spawn_instance` — start Roblox detached; optional priority,
   affinity, proxy env, custom integrations.
5. `watcher` child (`--watcher <pid>`) — tails the client log at 1 Hz:
   join/disconnect/teleport tracking, Discord presence, auto-rejoin,
   playtime, `ServerDetails.json` snapshot, on-close cleaning.

The watcher is a separate process so a UI crash can never take down launch
bookkeeping, and vice versa.

## Settings app (`desktop`)

Immediate-mode egui UI, one `RedStrapApp` struct holding all page state:

- `app.rs` — shell: nav, sidebar, toasts, dialogs, inbox pump (40 msgs/frame
  max), 500 ms repaint heartbeat, close-to-tray.
- `tasks.rs` — every blocking/network job runs on a Tokio runtime and
  reports back through `TaskMsg`; pages never block the UI thread.
- `tray.rs` — system-tray icon, menu, event polling.
- `ui/pages/` — ten pages; each `show(app, ui)` renders and mutates directly,
  saving atomically on every change.

## Self-updates

GitHub releases carry two assets: `RedStrap.exe` and
`RedStrap-Settings.exe`. `core::updater` downloads to a `-new` sidecar and
renames over the target with a `.old` backup (restored on failure).
Background updates stage silently and apply on next start; the About page
offers download + staged restart, plus per-version skip.

## Conventions

- No `unwrap`/`expect` outside `#[cfg(test)]` tests; errors use
  `core::error::Error` with context.
- No polling loops except the 1 Hz log tail and the UI heartbeat; background
  work is task + message based.
- Every setting maps to real behaviour — no decorative toggles.
- `cargo fmt`, `clippy -D warnings`, tests: all green before merge (CI).
