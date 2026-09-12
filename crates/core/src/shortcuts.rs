//! Managed shortcuts: launcher entries plus per-game join links.
//!
//! The launcher entries (Player / Studio / Settings on the desktop and the
//! Start Menu) follow the settings toggles. Game shortcuts join a specific
//! place with the game's own icon, fetched from Roblox and wrapped as ICO.

use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::paths::Layout;
use crate::settings::Settings;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Managed launcher shortcuts
// ---------------------------------------------------------------------------

/// (settings flag, file stem, launch args, description)
fn launcher_entries() -> [(&'static str, &'static str, &'static str); 3] {
    [
        ("player", "RedStrap Player", "--player"),
        ("studio", "RedStrap Studio", "--studio"),
        ("settings", "RedStrap Settings", "--settings"),
    ]
}

fn entry_enabled(settings: &Settings, key: &str) -> bool {
    match key {
        "player" => settings.shortcut_player,
        "studio" => settings.shortcut_studio,
        "settings" => settings.shortcut_settings,
        _ => false,
    }
}

/// Create or delete every managed shortcut to match the settings toggles.
pub fn sync_managed(layout: &Layout, settings: &Settings) -> Result<()> {
    let exe = layout.application.to_string_lossy().into_owned();
    let mut locations: Vec<(bool, PathBuf, &'static str)> = Vec::new();
    if let Some(desktop) = crate::util::desktop_dir() {
        locations.push((settings.shortcut_desktop, desktop, "desktop"));
    }
    #[cfg(windows)]
    if let Some(menu) = crate::util::start_menu_programs_dir() {
        let dir = menu.join("RedStrap");
        locations.push((settings.shortcut_start_menu, dir, "start menu"));
    }
    #[cfg(not(windows))]
    if settings.shortcut_start_menu {
        if let Some(home) = crate::util::home_dir() {
            // freedesktop application directory.
            locations.push((true, home.join(".local/share/applications"), "applications"));
        }
    }

    for (key, stem, args) in launcher_entries() {
        let wanted = entry_enabled(settings, key);
        for (location_on, dir, _label) in &locations {
            let active = wanted && *location_on;
            #[cfg(windows)]
            {
                let path = dir.join(format!("{stem}.lnk"));
                if active {
                    let spec = crate::shortcut::ShortcutSpec {
                        target: exe.clone(),
                        args: args.to_string(),
                        working_dir: layout.base.to_string_lossy().into_owned(),
                        icon: None,
                        description: Some(format!("{stem} (Red Strap)")),
                    };
                    if let Err(e) = crate::shortcut::write_lnk(&path, &spec) {
                        tracing::warn!("could not write {}: {e}", path.display());
                    }
                } else if path.is_file() {
                    let _ = std::fs::remove_file(&path);
                }
            }
            #[cfg(not(windows))]
            {
                let path = dir.join(format!("{stem}.desktop"));
                if active {
                    if let Err(e) = write_desktop_entry(&path, stem, &exe, args) {
                        tracing::warn!("could not write {}: {e}", path.display());
                    }
                } else if path.is_file() {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
    Ok(())
}

/// Remove every managed shortcut regardless of toggles (for uninstall).
pub fn remove_managed(layout: &Layout, settings: &Settings) -> Result<()> {
    let mut off = settings.clone();
    off.shortcut_player = false;
    off.shortcut_studio = false;
    off.shortcut_settings = false;
    // Keep the location flags so both spots are swept.
    sync_managed(layout, &off)
}

#[cfg(not(windows))]
fn write_desktop_entry(path: &std::path::Path, name: &str, exe: &str, args: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| Error::with_path(&parent.to_path_buf(), e))?;
        }
    }
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{exe}\" {args}\nTerminal=false\nCategories=Game;\n"
    );
    std::fs::write(path, entry).map_err(|e| Error::with_path(&path.to_path_buf(), e))
}

// ---------------------------------------------------------------------------
// Game shortcuts
// ---------------------------------------------------------------------------

/// A saved per-game join link.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameShortcut {
    pub name: String,
    pub place_id: u64,
    pub job_id: String,
    pub access_code: String,
    #[serde(default)]
    pub icon_file: String,
    #[serde(default)]
    pub link_file: String,
}

/// Create a desktop shortcut that joins `place_id` (optionally a specific
/// server), with the game's icon. Returns the `.lnk` path.
pub async fn create_game_shortcut(
    client: &reqwest::Client,
    layout: &Layout,
    settings: &Settings,
    name: &str,
    place_id: u64,
    job_id: &str,
    access_code: &str,
) -> Result<PathBuf> {
    if place_id == 0 {
        return Err(Error::Shortcut(String::from("place ID must not be zero")));
    }
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Shortcut(String::from("shortcut name must not be empty")));
    }

    let deeplink = crate::roblox::start_deeplink(place_id, job_id, access_code);
    let stem = crate::util::sanitize_filename(name);
    std::fs::create_dir_all(&layout.game_shortcuts)
        .map_err(|e| Error::with_path(&layout.game_shortcuts.clone(), e))?;

    // Best-effort icon: game thumbnail -> 256px PNG -> ICO wrapper.
    let icon_path = layout.game_shortcuts.join(format!("{stem}.ico"));
    let icon_ok = fetch_game_icon(client, settings, place_id, &icon_path).await;

    let desktop = crate::util::desktop_dir()
        .ok_or_else(|| Error::Shortcut(String::from("could not locate the desktop")))?;
    let exe = layout.application.to_string_lossy().into_owned();

    #[cfg(windows)]
    let link_path = {
        let link_path = desktop.join(format!("{stem}.lnk"));
        let spec = crate::shortcut::ShortcutSpec {
            target: exe,
            args: deeplink,
            working_dir: layout.base.to_string_lossy().into_owned(),
            icon: icon_ok.then(|| icon_path.to_string_lossy().into_owned()),
            description: Some(format!("Play {name} (Red Strap)")),
        };
        crate::shortcut::write_lnk(&link_path, &spec)?;
        link_path
    };
    #[cfg(not(windows))]
    let link_path = {
        let link_path = desktop.join(format!("{stem}.desktop"));
        // Quote the deeplink so the shell keeps it as one argument.
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{exe}\" \"{deeplink}\"\nTerminal=false\nCategories=Game;\n"
        );
        std::fs::write(&link_path, entry).map_err(|e| Error::with_path(&link_path.clone(), e))?;
        link_path
    };

    // Persist the spec so the desktop app can list/delete it later.
    let record = GameShortcut {
        name: name.to_string(),
        place_id,
        job_id: job_id.to_string(),
        access_code: access_code.to_string(),
        icon_file: if icon_ok {
            icon_path.to_string_lossy().into_owned()
        } else {
            String::new()
        },
        link_file: link_path.to_string_lossy().into_owned(),
    };
    let record_path = layout.game_shortcuts.join(format!("{stem}.json"));
    crate::util::save_json(&record_path, &record)?;

    Ok(link_path)
}

/// Delete a game shortcut created by [`create_game_shortcut`].
pub fn delete_game_shortcut(layout: &Layout, record_file: &str) -> Result<()> {
    let record_path = layout.game_shortcuts.join(record_file);
    let bytes = std::fs::read(&record_path).map_err(|e| Error::with_path(&record_path.clone(), e))?;
    let record: GameShortcut = serde_json::from_slice(&bytes)?;
    for file in [record.link_file, record.icon_file] {
        if !file.is_empty() {
            let _ = std::fs::remove_file(file);
        }
    }
    std::fs::remove_file(&record_path).map_err(|e| Error::with_path(&record_path.clone(), e))?;
    Ok(())
}

/// List saved game-shortcut records.
pub fn list_game_shortcuts(layout: &Layout) -> Vec<(String, GameShortcut)> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&layout.game_shortcuts) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(record) = serde_json::from_slice::<GameShortcut>(&bytes) {
                out.push((name, record));
            }
        }
    }
    out.sort_by(|a, b| a.1.name.to_ascii_lowercase().cmp(&b.1.name.to_ascii_lowercase()));
    out
}

// ---------------------------------------------------------------------------
// Game icons (place -> universe -> thumbnail -> ICO)
// ---------------------------------------------------------------------------

/// Fetch the icon for `place_id` and store it as `dest_ico`.
/// Returns false (without failing) when any step is unavailable.
async fn fetch_game_icon(
    client: &reqwest::Client,
    settings: &Settings,
    place_id: u64,
    dest_ico: &std::path::Path,
) -> bool {
    let domain = settings.roblox_domain.trim();
    let domain = if domain.is_empty() { "roblox.com" } else { domain };

    // placeId -> universeId (public).
    #[derive(Deserialize)]
    struct UniverseResponse {
        #[serde(rename = "universeId", default)]
        universe_id: u64,
    }
    let universe_url = format!("https://apis.{domain}/universes/v1/places/{place_id}/universe");
    let universe: UniverseResponse =
        match crate::http::get_json(client, &universe_url).await {
            Ok(u) if u.universe_id > 0 => u,
            _ => return false,
        };

    #[derive(Deserialize)]
    struct ThumbResponse {
        #[serde(default)]
        data: Vec<ThumbEntry>,
    }
    #[derive(Deserialize)]
    struct ThumbEntry {
        #[serde(rename = "imageUrl", default)]
        image_url: String,
    }
    let thumbs_url = format!(
        "https://thumbnails.{domain}/v1/games/icons?universeIds={}&size=256x256&format=Png&isCircular=false",
        universe.universe_id
    );
    let thumbs: ThumbResponse = match crate::http::get_json(client, &thumbs_url).await {
        Ok(t) => t,
        Err(_) => return false,
    };
    let image_url = match thumbs.data.into_iter().next() {
        Some(e) if !e.image_url.is_empty() => e.image_url,
        _ => return false,
    };

    let bytes = match crate::http::get_bytes(client, &image_url).await {
        Ok(b) => b,
        Err(_) => return false,
    };
    if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
        return false;
    }

    // Decode, normalize to 256x256 RGBA, re-encode as PNG, wrap in ICO.
    // Blocking image work runs on the calling thread; the payload is tiny.
    let png = match make_icon_png(&bytes) {
        Some(p) => p,
        None => return false,
    };
    let ico = wrap_png_ico(&png);
    std::fs::write(dest_ico, ico).is_ok()
}

fn make_icon_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    let rgba = image.to_rgba8();
    let resized = if rgba.width() != 256 || rgba.height() != 256 {
        image::imageops::resize(&rgba, 256, 256, image::imageops::FilterType::Lanczos3)
    } else {
        rgba
    };
    let mut png = Vec::new();
    {
        use image::ImageEncoder;
        let encoder = image::codecs::png::PngEncoder::new(&mut png);
        encoder
            .write_image(
                resized.as_raw(),
                resized.width(),
                resized.height(),
                image::ExtendedColorType::Rgba8,
            )
            .ok()?;
    }
    Some(png)
}

/// Wrap PNG bytes as a single-entry ICO (Vista and later read PNG icons).
fn wrap_png_ico(png: &[u8]) -> Vec<u8> {
    let mut ico = Vec::with_capacity(22 + png.len());
    ico.extend_from_slice(&0u16.to_le_bytes()); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    ico.extend_from_slice(&1u16.to_le_bytes()); // count
    ico.push(0); // width 256 encoded as 0
    ico.push(0); // height 256 encoded as 0
    ico.push(0); // palette colors
    ico.push(0); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // color planes
    ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes()); // pixel data offset
    ico.extend_from_slice(png);
    ico
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ico_wrapper_layout() {
        let png = vec![1u8, 2, 3, 4];
        let ico = wrap_png_ico(&png);
        assert_eq!(ico.len(), 22 + 4);
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(u32::from_le_bytes([ico[14], ico[15], ico[16], ico[17]]), 4);
        assert_eq!(
            u32::from_le_bytes([ico[18], ico[19], ico[20], ico[21]]),
            22
        );
        assert_eq!(&ico[22..], &[1, 2, 3, 4]);
    }
}
