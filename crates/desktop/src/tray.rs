//! System tray icon with a small action menu.
//!
//! The tray lives on the UI thread and is polled once per frame — no extra
//! threads, no hidden windows. Left-click (or double-click) shows the
//! settings window; the menu offers quick launches, an update check, quit.

use muda::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Actions requested through the tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    LaunchPlayer,
    LaunchStudio,
    CheckUpdates,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    show: MenuItem,
    player: MenuItem,
    studio: MenuItem,
    update: MenuItem,
    quit: MenuItem,
}

impl Tray {
    /// Build the icon + menu. Fails with a plain message when the OS tray
    /// is unavailable (the app works fine without it).
    pub fn new() -> Result<Self, String> {
        let icon = load_icon()?;

        let menu = Menu::new();
        let show = MenuItem::new("Show Red Strap", true, None);
        let player = MenuItem::new("Launch Player", true, None);
        let studio = MenuItem::new("Launch Studio", true, None);
        let update = MenuItem::new("Check for Updates", true, None);
        let quit = MenuItem::new("Quit", true, None);
        menu.append(&show).map_err(|e| e.to_string())?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|e| e.to_string())?;
        menu.append(&player).map_err(|e| e.to_string())?;
        menu.append(&studio).map_err(|e| e.to_string())?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|e| e.to_string())?;
        menu.append(&update).map_err(|e| e.to_string())?;
        menu.append(&quit).map_err(|e| e.to_string())?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip("Red Strap")
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            _icon: tray,
            show,
            player,
            studio,
            update,
            quit,
        })
    }

    /// Drain pending tray/menu events into actions.
    pub fn poll(&self) -> Vec<TrayAction> {
        let mut out = Vec::new();

        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            match event {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    ..
                } => out.push(TrayAction::Show),
                TrayIconEvent::DoubleClick { .. } => out.push(TrayAction::Show),
                _ => {}
            }
        }

        while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
            let id = event.id();
            if id == self.show.id() {
                out.push(TrayAction::Show);
            } else if id == self.player.id() {
                out.push(TrayAction::LaunchPlayer);
            } else if id == self.studio.id() {
                out.push(TrayAction::LaunchStudio);
            } else if id == self.update.id() {
                out.push(TrayAction::CheckUpdates);
            } else if id == self.quit.id() {
                out.push(TrayAction::Quit);
            }
        }

        out
    }
}

fn load_icon() -> Result<Icon, String> {
    let image = image::load_from_memory(crate::ui::theme::ICON_PNG)
        .map_err(|e| format!("tray icon decode failed: {e}"))?;
    let rgba = image.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Icon::from_rgba(rgba.into_raw(), w, h).map_err(|e| format!("tray icon rejected: {e:?}"))
}
