//! Minimal Windows shortcut (`.lnk`) writer, dependency-free.
//!
//! Implements the subset of MS-SHLLINK needed for application shortcuts:
//! a `ShellLinkHeader`, `LinkInfo` with Unicode path extensions, `StringData`
//! (working directory, arguments, icon, description), and the terminal block.
//! No target ID list is emitted — Explorer resolves these links through
//! `LinkInfo`, which also keeps non-ASCII paths working via the Unicode
//! offsets. Verified by round-trip layout tests below.

use std::path::Path;

use crate::error::Result;

// LinkFlags bits.
const HAS_LINK_INFO: u32 = 0x0000_0002;
const HAS_NAME: u32 = 0x0000_0004;
const HAS_WORKING_DIR: u32 = 0x0000_0010;
const HAS_ARGUMENTS: u32 = 0x0000_0020;
const HAS_ICON_LOCATION: u32 = 0x0000_0040;
const IS_UNICODE: u32 = 0x0000_0080;

// LinkInfoFlags bits.
const VOLUME_ID_AND_LOCAL_BASE_PATH: u32 = 0x0000_0001;

/// `00021401-0000-0000-C000-000000000046` in mixed-endian binary form.
const LINK_CLSID: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x46,
];

/// Shortcut definition. All paths accept `/` or `\` separators.
#[derive(Debug, Clone)]
pub struct ShortcutSpec {
    /// Executable the shortcut launches.
    pub target: String,
    /// Command-line arguments (may be empty).
    pub args: String,
    /// Working directory (defaults to the target's folder).
    pub working_dir: String,
    /// Icon source (`None` reuses the target's own icon).
    pub icon: Option<String>,
    /// Description shown in tooltips / properties.
    pub description: Option<String>,
}

impl ShortcutSpec {
    pub fn new(target: &str) -> Self {
        let working_dir = target
            .rsplit(['/', '\\'])
            .nth(1)
            .unwrap_or("")
            .to_string();
        Self {
            target: target.to_string(),
            args: String::new(),
            working_dir,
            icon: None,
            description: None,
        }
    }
}

/// Build `.lnk` file bytes for `spec`.
pub fn build_lnk(spec: &ShortcutSpec) -> Vec<u8> {
    let target = to_windows_path(&spec.target);
    let file_name = target
        .rsplit('\\')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(&target)
        .to_string();
    let working_dir = if spec.working_dir.trim().is_empty() {
        target
            .rsplit('\\')
            .nth(1)
            .unwrap_or("C:\\")
            .to_string()
    } else {
        to_windows_path(&spec.working_dir)
    };

    let mut flags = HAS_LINK_INFO | IS_UNICODE;
    if spec.description.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
        flags |= HAS_NAME;
    }
    if !working_dir.is_empty() {
        flags |= HAS_WORKING_DIR;
    }
    if !spec.args.is_empty() {
        flags |= HAS_ARGUMENTS;
    }
    if spec.icon.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
        flags |= HAS_ICON_LOCATION;
    }

    let mut out = Vec::with_capacity(512);
    push_header(&mut out, flags);
    push_link_info(&mut out, &target, &file_name);
    // StringData order is fixed: Name, RelativePath, WorkingDir, Arguments,
    // IconLocation. (RelativePath is never emitted.)
    if flags & HAS_NAME != 0 {
        push_string_data(&mut out, spec.description.as_deref().unwrap_or(""));
    }
    if flags & HAS_WORKING_DIR != 0 {
        push_string_data(&mut out, &working_dir);
    }
    if flags & HAS_ARGUMENTS != 0 {
        push_string_data(&mut out, &spec.args);
    }
    if flags & HAS_ICON_LOCATION != 0 {
        // IconLocation format is "path,index".
        let icon = spec.icon.as_deref().unwrap_or("");
        let value = if icon.contains(',') {
            icon.to_string()
        } else {
            format!("{icon},0")
        };
        push_string_data(&mut out, &value);
    }
    // TerminalBlock: a zero-size ExtraData block.
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

/// Write a `.lnk` file, overwriting any previous one.
pub fn write_lnk(path: &Path, spec: &ShortcutSpec) -> Result<()> {
    if spec.target.trim().is_empty() {
        return Err(crate::error::Error::Shortcut(String::from(
            "shortcut target must not be empty",
        )));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| crate::error::Error::with_path(&parent.to_path_buf(), e))?;
        }
    }
    let bytes = build_lnk(spec);
    std::fs::write(path, bytes).map_err(|e| crate::error::Error::with_path(&path.to_path_buf(), e))
}

fn to_windows_path(path: &str) -> String {
    path.replace('/', "\\")
}

fn push_header(out: &mut Vec<u8>, flags: u32) {
    out.extend_from_slice(&76u32.to_le_bytes()); // HeaderSize
    out.extend_from_slice(&LINK_CLSID);
    out.extend_from_slice(&flags.to_le_bytes()); // LinkFlags
    out.extend_from_slice(&0u32.to_le_bytes()); // FileAttributes
    out.extend_from_slice(&0u64.to_le_bytes()); // CreationTime
    out.extend_from_slice(&0u64.to_le_bytes()); // AccessTime
    out.extend_from_slice(&0u64.to_le_bytes()); // WriteTime
    out.extend_from_slice(&0u32.to_le_bytes()); // FileSize
    out.extend_from_slice(&0i32.to_le_bytes()); // IconIndex
    out.extend_from_slice(&1u32.to_le_bytes()); // ShowCommand = SW_SHOWNORMAL
    out.extend_from_slice(&0u16.to_le_bytes()); // HotKey
    out.extend_from_slice(&0u16.to_le_bytes()); // Reserved1
    out.extend_from_slice(&0u32.to_le_bytes()); // Reserved2
    out.extend_from_slice(&0u32.to_le_bytes()); // Reserved3
}

fn push_link_info(out: &mut Vec<u8>, full_path: &str, file_name: &str) {
    // ANSI copies are lossy; Unicode copies are authoritative.
    let ansi_base: Vec<u8> = full_path
        .bytes()
        .map(|b| if b.is_ascii() { b } else { b'?' })
        .chain(std::iter::once(0))
        .collect();
    let ansi_suffix: Vec<u8> = file_name
        .bytes()
        .map(|b| if b.is_ascii() { b } else { b'?' })
        .chain(std::iter::once(0))
        .collect();
    let uni_base: Vec<u8> = full_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|c| c.to_le_bytes())
        .collect();
    let uni_suffix: Vec<u8> = file_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|c| c.to_le_bytes())
        .collect();

    // Minimal VolumeID: fixed drive, no label.
    let volume_id: Vec<u8> = {
        let mut v = Vec::with_capacity(17);
        v.extend_from_slice(&17u32.to_le_bytes()); // VolumeIDSize
        v.extend_from_slice(&3u32.to_le_bytes()); // DriveType = fixed
        v.extend_from_slice(&0u32.to_le_bytes()); // DriveSerialNumber
        v.extend_from_slice(&16u32.to_le_bytes()); // VolumeLabelOffset
        v.push(0); // empty label + NUL
        v
    };

    // Offsets are relative to the start of LinkInfo.
    let header_len = 36u32;
    let volume_off = header_len;
    let base_off = volume_off + volume_id.len() as u32;
    let suffix_off = base_off + ansi_base.len() as u32;
    let uni_base_off = suffix_off + ansi_suffix.len() as u32;
    let uni_suffix_off = uni_base_off + uni_base.len() as u32;
    let total = uni_suffix_off + uni_suffix.len() as u32;

    out.extend_from_slice(&total.to_le_bytes()); // LinkInfoSize
    out.extend_from_slice(&header_len.to_le_bytes()); // LinkInfoHeaderSize
    out.extend_from_slice(&VOLUME_ID_AND_LOCAL_BASE_PATH.to_le_bytes());
    out.extend_from_slice(&volume_off.to_le_bytes()); // VolumeIDOffset
    out.extend_from_slice(&base_off.to_le_bytes()); // LocalBasePathOffset
    out.extend_from_slice(&0u32.to_le_bytes()); // CommonNetworkRelativeLinkOffset
    out.extend_from_slice(&suffix_off.to_le_bytes()); // CommonPathSuffixOffset
    out.extend_from_slice(&uni_base_off.to_le_bytes()); // LocalBasePathOffsetUnicode
    out.extend_from_slice(&uni_suffix_off.to_le_bytes()); // CommonPathSuffixOffsetUnicode
    out.extend_from_slice(&volume_id);
    out.extend_from_slice(&ansi_base);
    out.extend_from_slice(&ansi_suffix);
    out.extend_from_slice(&uni_base);
    out.extend_from_slice(&uni_suffix);
}

fn push_string_data(out: &mut Vec<u8>, text: &str) {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    // CountCharacters is capped at u16::MAX; longer strings are truncated.
    let count = utf16.len().min(u16::MAX as usize);
    out.extend_from_slice(&(count as u16).to_le_bytes());
    for unit in utf16.iter().take(count) {
        out.extend_from_slice(&unit.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u16(data: &[u8], off: usize) -> u16 {
        u16::from_le_bytes([data[off], data[off + 1]])
    }

    fn read_u32(data: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
    }

    fn read_wstring(data: &[u8], off: usize) -> String {
        let end = data[off..]
            .chunks_exact(2)
            .position(|c| c == [0, 0])
            .map(|p| off + p * 2)
            .unwrap_or(data.len());
        let units: Vec<u16> = data[off..end]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    }

    #[test]
    fn layout_roundtrips() {
        let spec = ShortcutSpec {
            target: String::from("C:/Apps/RedStrap/RedStrap.exe"),
            args: String::from("--settings"),
            working_dir: String::from("C:\\Apps\\RedStrap"),
            icon: Some(String::from("C:\\Apps\\RedStrap\\game.ico")),
            description: Some(String::from("Red Strap settings")),
        };
        let lnk = build_lnk(&spec);

        // Header.
        assert_eq!(read_u32(&lnk, 0), 76);
        assert_eq!(&lnk[4..20], &LINK_CLSID);
        let flags = read_u32(&lnk, 20);
        assert_ne!(flags & HAS_LINK_INFO, 0);
        assert_ne!(flags & IS_UNICODE, 0);
        assert_ne!(flags & HAS_ARGUMENTS, 0);
        assert_ne!(flags & HAS_ICON_LOCATION, 0);

        // LinkInfo offsets resolve to the right strings.
        let info_off = 76usize;
        let total = read_u32(&lnk, info_off) as usize;
        assert_eq!(read_u32(&lnk, info_off + 4), 36);
        let uni_base = read_u32(&lnk, info_off + 28) as usize;
        let uni_suffix = read_u32(&lnk, info_off + 32) as usize;
        assert_eq!(
            read_wstring(&lnk, info_off + uni_base),
            "C:\\Apps\\RedStrap\\RedStrap.exe"
        );
        assert_eq!(read_wstring(&lnk, info_off + uni_suffix), "RedStrap.exe");

        // StringData section.
        let mut off = info_off + total;
        let mut strings = Vec::new();
        // Name, WorkingDir, Arguments, IconLocation (in order).
        for _ in 0..4 {
            let count = read_u16(&lnk, off) as usize;
            off += 2;
            let units: Vec<u16> = lnk[off..off + count * 2]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            strings.push(String::from_utf16_lossy(&units));
            off += count * 2;
        }
        assert_eq!(strings[0], "Red Strap settings");
        assert_eq!(strings[1], "C:\\Apps\\RedStrap");
        assert_eq!(strings[2], "--settings");
        assert_eq!(strings[3], "C:\\Apps\\RedStrap\\game.ico,0");

        // Terminal block.
        assert_eq!(read_u32(&lnk, off), 0);
        assert_eq!(off + 4, lnk.len());
    }

    #[test]
    fn minimal_shortcut_has_no_optional_strings() {
        let spec = ShortcutSpec::new("C:\\Apps\\RedStrap.exe");
        let lnk = build_lnk(&spec);
        let flags = read_u32(&lnk, 20);
        assert_eq!(flags & HAS_ARGUMENTS, 0);
        assert_eq!(flags & HAS_ICON_LOCATION, 0);
        assert_eq!(flags & HAS_NAME, 0);
        assert_ne!(flags & HAS_WORKING_DIR, 0);
    }
}
