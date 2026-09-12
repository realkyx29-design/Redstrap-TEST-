//! File mods: discovery, installation, application, and recoloring.
//!
//! A mod is a folder under `Modifications/` whose contents mirror Roblox's
//! version directory (`content/textures/...`). Enabled mods are copied over
//! the installed version after every update. The recolor tool tints known
//! UI textures (cursors, shift-lock, emote wheel) while preserving alpha.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::util::sanitize_filename;

// ---------------------------------------------------------------------------
// Discovery & application
// ---------------------------------------------------------------------------

/// A mod folder found under `Modifications/`.
#[derive(Debug, Clone)]
pub struct ModDir {
    /// Folder name (also the key in [`crate::state::State::mods`]).
    pub name: String,
    pub path: PathBuf,
    pub file_count: usize,
    pub total_bytes: u64,
}

/// Scan `mods_dir` for mod folders (non-recursive top level).
pub fn scan_mods(mods_dir: &Path) -> Vec<ModDir> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(mods_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip marker/config files that live alongside mods.
        if name.starts_with('.') {
            continue;
        }
        let (file_count, total_bytes) = dir_stats(&path);
        out.push(ModDir {
            name,
            path,
            file_count,
            total_bytes,
        });
    }
    out.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
    out
}

fn dir_stats(dir: &Path) -> (usize, u64) {
    let mut count = 0;
    let mut bytes = 0;
    let mut stack = vec![dir.to_path_buf()];
    // Iterative walk with a depth cap to survive pathological trees.
    let mut guard = 0;
    while let Some(next) = stack.pop() {
        guard += 1;
        if guard > 50_000 {
            break;
        }
        let entries = match std::fs::read_dir(&next) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                count += 1;
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    (count, bytes)
}

/// Copy every file from each enabled mod directory into `dest_version_dir`,
/// preserving relative paths. Returns the number of files copied.
///
/// Files that fail to copy are collected into the returned error list rather
/// than aborting the whole pass, so one locked file cannot break a launch.
pub fn apply_mods(mod_dirs: &[PathBuf], dest_version_dir: &Path) -> (usize, Vec<String>) {
    let mut copied = 0;
    let mut failures = Vec::new();
    for dir in mod_dirs {
        let (n, mut errs) = copy_tree(dir, dest_version_dir);
        copied += n;
        failures.append(&mut errs);
    }
    (copied, failures)
}

fn copy_tree(src_root: &Path, dest_root: &Path) -> (usize, Vec<String>) {
    let mut copied = 0;
    let mut failures = Vec::new();
    let mut stack = vec![src_root.to_path_buf()];
    let mut guard = 0;
    while let Some(next) = stack.pop() {
        guard += 1;
        if guard > 100_000 {
            failures.push(format!("{}: directory tree too deep, stopped", src_root.display()));
            break;
        }
        let entries = match std::fs::read_dir(&next) {
            Ok(e) => e,
            Err(e) => {
                failures.push(format!("{}: {e}", next.display()));
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let relative = match path.strip_prefix(src_root) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if relative.as_os_str().is_empty() {
                continue;
            }
            let dest = dest_root.join(relative);
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                if std::fs::create_dir_all(&dest).is_err() {
                    failures.push(format!("{}: could not create directory", dest.display()));
                } else {
                    stack.push(path);
                }
            } else if file_type.is_file() {
                if let Some(parent) = dest.parent() {
                    if std::fs::create_dir_all(parent).is_err() {
                        failures.push(format!("{}: could not create directory", dest.display()));
                        continue;
                    }
                }
                match std::fs::copy(&path, &dest) {
                    Ok(_) => copied += 1,
                    Err(e) => failures.push(format!("{}: {e}", dest.display())),
                }
            }
            // Symlinks and other special files are skipped deliberately:
            // mods must be self-contained file trees.
        }
    }
    (copied, failures)
}

// ---------------------------------------------------------------------------
// Installation
// ---------------------------------------------------------------------------

/// Install a `.zip` as a new mod folder. `wanted_name` is sanitized and
/// de-duplicated (`name`, `name (2)`, ...). ZipSlip-unsafe entries abort
/// the whole install — archives must be well-formed.
pub fn install_from_zip(
    zip_path: &Path,
    mods_dir: &Path,
    wanted_name: &str,
) -> Result<PathBuf> {
    let file =
        std::fs::File::open(zip_path).map_err(|e| Error::with_path(&zip_path.to_path_buf(), e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| Error::Mod(format!("could not open archive: {e}")))?;
    if archive.is_empty() {
        return Err(Error::Mod(String::from("archive contains no files")));
    }

    let dest = unique_mod_dir(mods_dir, wanted_name)?;
    std::fs::create_dir_all(&dest).map_err(|e| Error::with_path(&dest.clone(), e))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| Error::Mod(format!("could not read archive entry: {e}")))?;
        let relative = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => {
                let _ = std::fs::remove_dir_all(&dest);
                return Err(Error::Mod(format!(
                    "archive entry '{}' would escape the mod folder",
                    entry.name()
                )));
            }
        };
        let out_path = dest.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| Error::with_path(&out_path.clone(), e))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::with_path(&parent.to_path_buf(), e))?;
        }
        let mut out_file =
            std::fs::File::create(&out_path).map_err(|e| Error::with_path(&out_path.clone(), e))?;
        std::io::copy(&mut entry, &mut out_file)
            .map_err(|e| Error::with_path(&out_path.clone(), e))?;
    }

    Ok(dest)
}

/// Install loose files / folders as a new mod folder.
pub fn install_from_paths(
    sources: &[PathBuf],
    mods_dir: &Path,
    wanted_name: &str,
) -> Result<PathBuf> {
    if sources.is_empty() {
        return Err(Error::Mod(String::from("nothing selected to install")));
    }
    let dest = unique_mod_dir(mods_dir, wanted_name)?;
    std::fs::create_dir_all(&dest).map_err(|e| Error::with_path(&dest.clone(), e))?;
    let mut installed = 0;
    for src in sources {
        if src.is_dir() {
            let (n, _) = copy_tree(src, &dest);
            installed += n;
        } else if src.is_file() {
            let name = src
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file");
            let out_path = dest.join(sanitize_filename(name));
            std::fs::copy(src, &out_path).map_err(|e| Error::with_path(&out_path.clone(), e))?;
            installed += 1;
        }
    }
    if installed == 0 {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(Error::Mod(String::from(
            "none of the selected items could be installed",
        )));
    }
    Ok(dest)
}

fn unique_mod_dir(mods_dir: &Path, wanted_name: &str) -> Result<PathBuf> {
    let base = sanitize_filename(wanted_name);
    let mut candidate = mods_dir.join(&base);
    if !candidate.exists() {
        return Ok(candidate);
    }
    for n in 2..1000 {
        candidate = mods_dir.join(format!("{base} ({n})"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(Error::Mod(String::from(
        "could not find a free folder name for this mod",
    )))
}

// ---------------------------------------------------------------------------
// Recoloring
// ---------------------------------------------------------------------------

/// A group of tintable UI textures inside an installed version.
#[derive(Debug, Clone, Copy)]
pub struct RecolorGroup {
    pub id: &'static str,
    pub label: &'static str,
    pub dir: &'static str,
    pub files: &'static [&'static str],
}

/// Built-in recolor targets (paths relative to the version directory).
pub const RECOLOR_GROUPS: &[RecolorGroup] = &[
    RecolorGroup {
        id: "cursors",
        label: "Mouse cursors",
        dir: "content/textures/Cursors/KeyboardMouse",
        files: &["IBeamCursor.png", "ArrowCursor.png", "ArrowFarCursor.png"],
    },
    RecolorGroup {
        id: "shiftlock",
        label: "Shift-lock cursor",
        dir: "content/textures",
        files: &["MouseLockedCursor.png"],
    },
    RecolorGroup {
        id: "emote",
        label: "Emote wheel selection",
        dir: "content/textures/ui/Emotes/Large",
        files: &[
            "SelectedGradient.png",
            "SelectedGradient@2x.png",
            "SelectedGradient@3x.png",
            "SelectedLine.png",
            "SelectedLine@2x.png",
            "SelectedLine@3x.png",
        ],
    },
];

/// Outcome of a recolor pass.
#[derive(Debug, Clone, Default)]
pub struct RecolorReport {
    pub recolored: usize,
    pub missing: Vec<String>,
    pub failed: Vec<String>,
}

/// Parse `#rgb` / `#rrggbb` / `rrggbb` into RGB bytes.
pub fn parse_hex_color(text: &str) -> Option<(u8, u8, u8)> {
    let hex = text.trim().strip_prefix('#').unwrap_or_else(|| text.trim());
    let bytes = hex.as_bytes();
    let pair = |i: usize| -> Option<u8> {
        u8::from_str_radix(std::str::from_utf8(bytes.get(i..i + 2)?).ok()?, 16).ok()
    };
    match bytes.len() {
        3 => {
            let digit = |i: usize| -> Option<u8> {
                let c = *bytes.get(i)? as char;
                let n = c.to_digit(16)? as u8;
                Some(n * 16 + n)
            };
            Some((digit(0)?, digit(1)?, digit(2)?))
        }
        6 => Some((pair(0)?, pair(2)?, pair(4)?)),
        _ => None,
    }
}

/// Tint PNGs under `version_dir`: the selected built-in groups plus optional
/// extra relative paths. RGB channels are replaced while alpha is preserved,
/// so shapes and soft edges survive. Returns per-file outcomes.
pub fn recolor_pngs(
    version_dir: &Path,
    color: (u8, u8, u8),
    groups: &[RecolorGroup],
    extra_relative_paths: &[String],
) -> RecolorReport {
    let mut report = RecolorReport::default();
    let mut targets = Vec::new();
    for group in groups {
        for file in group.files {
            targets.push(format!("{}/{}", group.dir, file));
        }
    }
    for extra in extra_relative_paths {
        let clean = extra.trim().trim_matches(['/', '\\']).replace('\\', "/");
        if clean.is_empty() || clean.contains("..") {
            report.failed.push(format!("{extra}: rejected unsafe path"));
            continue;
        }
        targets.push(clean);
    }

    for target in targets {
        let path = version_dir.join(&target);
        if !path.is_file() {
            report.missing.push(target);
            continue;
        }
        match tint_png(&path, color) {
            Ok(()) => report.recolored += 1,
            Err(e) => report.failed.push(format!("{target}: {e}")),
        }
    }
    report
}

fn tint_png(path: &Path, color: (u8, u8, u8)) -> Result<()> {
    let bytes = std::fs::read(path).map_err(|e| Error::with_path(&path.to_path_buf(), e))?;
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?;
    let mut rgba = image.to_rgba8();
    for pixel in rgba.pixels_mut() {
        pixel[0] = color.0;
        pixel[1] = color.1;
        pixel[2] = color.2;
    }
    let mut encoded = Vec::new();
    {
        use image::ImageEncoder;
        let encoder = image::codecs::png::PngEncoder::new(&mut encoded);
        encoder
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| Error::Mod(format!("could not encode PNG: {e}")))?;
    }
    crate::util::atomic_write(path, &encoded)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_parse() {
        assert_eq!(parse_hex_color("#e11d2e"), Some((0xe1, 0x1d, 0x2e)));
        assert_eq!(parse_hex_color("e11d2e"), Some((0xe1, 0x1d, 0x2e)));
        assert_eq!(parse_hex_color("#abc"), Some((0xaa, 0xbb, 0xcc)));
        assert_eq!(parse_hex_color("#abcd"), None);
        assert_eq!(parse_hex_color("not a color"), None);
    }

    #[test]
    fn scan_and_apply_roundtrip() {
        let root = std::env::temp_dir().join(format!("redstrap-mod-test-{}", std::process::id()));
        let mods = root.join("mods");
        let mine = mods.join("mine");
        std::fs::create_dir_all(mine.join("content")).expect("mkdir");
        std::fs::write(mine.join("content/a.txt"), b"hello").expect("write");

        let found = scan_mods(&mods);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_count, 1);

        let dest = root.join("version");
        let (copied, failures) = apply_mods(&[mine], &dest);
        assert_eq!(copied, 1);
        assert!(failures.is_empty());
        assert_eq!(
            std::fs::read(dest.join("content/a.txt")).expect("read"),
            b"hello"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
