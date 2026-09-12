<h1 align="center">Red Strap</h1>

<p align="center">
  A fast, lightweight Roblox bootstrapper and mod manager — rewritten in Rust.
</p>

<p align="center">
  Forked from <a href="https://github.com/Froststrap/Froststrap">Froststrap</a>.
</p>

<p align="center">
  <img src="./assets/icon/icon.png" height="128" alt="Red Strap logo"/>
</p>

<p align="center">
  If you'd like to support the project, consider giving this repository a star!
</p>

> [!CAUTION]
> Download Red Strap **only** from this repository's
> [releases](https://github.com/realkyx29-design/Redstrap-TEST-/releases).
> Binaries from any other source are **not** affiliated with us and may be malicious.

---

## What is Red Strap?

Red Strap replaces the default Roblox launcher with a bootstrapper that is:

- **Fast** — installs, patches, and launches with minimal overhead; the
  settings UI is a native app that starts instantly and idles at ~0% CPU.
- **Customizable** — Fast Flag editor with profiles and allowlist validation,
  file mods, cursor sets, UI recoloring, custom fonts, and per-game shortcuts.
- **Observable** — live session tracking, playtime counter, Discord Rich
  Presence (including in-game `[RedStrapRPC]`), and a built-in log viewer.
- **Low-maintenance** — silent background self-updates, channel management,
  and one-click reinstall/repair.

Red Strap is a clean-room Rust rewrite inspired by tools like
[Bloxstrap](https://github.com/bloxstraplabs/bloxstrap). It keeps the workflow
you know (deploy Roblox, apply your tweaks, launch, watch) while rebuilding
every layer for speed and reliability.

## Screenshots

> Screenshots coming soon — the UI is a native red + dark desktop app with ten
> settings pages: Overview, Launch, Performance, Fast Flags, Mods,
> Integrations, Shortcuts, Appearance, Logs, and About.

## Download

Grab `RedStrap.exe` from the
[latest release](https://github.com/realkyx29-design/Redstrap-TEST-/releases),
run it, and Roblox will install and launch. The settings UI
(`RedStrap-Settings.exe`) ships alongside it and opens from the launcher,
the Start Menu shortcut, or the system tray.

Requirements: **Windows 10/11 x64**. (The core builds on Linux/macOS for
development, but launching Roblox itself is Windows-only.)

## Features

| Area | Highlights |
| --- | --- |
| Deploy | Channel + version pinning, parallel downloads with resume, hash verification, staging, automatic repair |
| Launch | Player/Studio, custom args, multi-instance, process priority, CPU affinity, proxy support |
| Performance | FPS cap, rendering backend (Vulkan/OpenGL/D3D11), MSAA, graphics-mode flags, GBS overrides, Windows graphics preference |
| Fast Flags | Search, undo/redo, profiles, import/export, community allowlist validation |
| Mods | Drag-and-drop ZIP/folder install, per-mod Player/Studio toggles, UI recolor, cursor sets |
| Presence | Discord Rich Presence with game name, server buttons, account display, and game-driven `[RedStrapRPC]` |
| Automation | Auto-rejoin dropped servers, close-on-leave, playtime counter, external-program integrations |
| Updates | Stable/pre-release channels, background updates, skip-this-version, staged restart |

## Building from source

Prerequisites: [Rust stable](https://rustup.rs/) 1.76+ (with `rustfmt` +
`clippy`), and on Linux the GUI system libraries (see `ci.yml`).

```powershell
# Debug build
cargo build --workspace

# Optimized release binaries (target/release/RedStrap.exe, RedStrap-Settings.exe)
cargo build --workspace --release

# Tests, lints, formatting (this is what CI runs)
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

[`just`](https://github.com/casey/just) shortcuts are available too:
`just build`, `just release`, `just check`, `just clean`.

### Repository layout

```text
crates/
  core/        Shared library: settings, deployment pipeline, fast flags,
               mods, Roblox protocol/log parsing, Discord IPC, self-updater
  bootstrap/   RedStrap.exe — CLI launcher, installer, session watcher
  desktop/     RedStrap-Settings.exe — native egui settings app
assets/        Application icon (PNG + ICO)
docs/          Architecture and development notes
```

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for how the pieces fit
together.

## Configuration

All state lives under `%LocalAppData%\RedStrap`:

- `Settings.json` — every option the UI exposes (hand-editable; corrupt
  files are backed up and reset, never crash).
- `State.json` — UI memory, installed mods, playtime, skipped versions.
- `ClientAppSettings.json` — the generated fast-flag file Roblox reads.

Logs rotate daily under `Logs/` and are viewable live on the Logs page.
Set `RUST_LOG` (e.g. `RUST_LOG=debug`) to override the log level, or enable
*Verbose logging* on the Logs page.

## Contributing

Issues and pull requests are welcome. Please run `just check` (or the
equivalent cargo commands above) before opening a PR — CI denies warnings.

## License

Red Strap is licensed under the [MIT License](LICENSE-MIT). See `LICENSE*`
for the full texts.
