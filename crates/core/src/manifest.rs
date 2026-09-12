//! Parsers for Roblox deployment manifests.
//!
//! A version's `rbxPkgManifest.txt` lists every downloadable package as
//! repeating 4-line groups: file name, MD5 signature, packed size, and
//! extracted size. The legacy `RobloxPlayerLauncher.exe` entry terminates
//! the list and is skipped, matching long-standing bootstrapper behaviour.

use crate::error::{Error, Result};

/// A single downloadable package from a version manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// Zip file name, e.g. `RobloxApp.zip`.
    pub name: String,
    /// Lowercase hex MD5 of the packed file; also its cache file name.
    pub signature: String,
    /// Compressed size in bytes.
    pub packed_size: u64,
    /// Extracted size in bytes.
    pub size: u64,
}

/// Parse the text of a `*-rbxPkgManifest.txt` file.
pub fn parse_package_manifest(text: &str) -> Result<Vec<Package>> {
    let mut lines = text.lines();

    match lines.next() {
        Some("v0") => {}
        Some(other) => {
            return Err(Error::Manifest(format!(
                "unsupported package manifest version '{other}' (expected 'v0')"
            )))
        }
        None => return Err(Error::Manifest(String::from("manifest is empty"))),
    }

    let mut packages = Vec::new();

    loop {
        let name = lines.next().unwrap_or("").trim();
        let signature = lines.next().unwrap_or("").trim();
        let packed = lines.next().unwrap_or("").trim();
        let size = lines.next().unwrap_or("").trim();

        if name.is_empty() || signature.is_empty() || packed.is_empty() || size.is_empty() {
            break;
        }

        // The legacy launcher entry ends the list; it is never downloaded.
        if name.eq_ignore_ascii_case("RobloxPlayerLauncher.exe") {
            break;
        }

        let packed_size = packed.parse::<u64>().map_err(|_| {
            Error::Manifest(format!("invalid packed size '{packed}' for package '{name}'"))
        })?;
        let size = size.parse::<u64>().map_err(|_| {
            Error::Manifest(format!("invalid size '{size}' for package '{name}'"))
        })?;

        packages.push(Package {
            name: name.to_string(),
            signature: signature.to_ascii_lowercase(),
            packed_size,
            size,
        });
    }

    if packages.is_empty() {
        return Err(Error::Manifest(String::from(
            "manifest contains no downloadable packages",
        )));
    }

    Ok(packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "v0\nRobloxApp.zip\n900150983cd24fb0d6963f7d28e17f72\n100\n200\nExtra.zip\nAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n50\n60\nRobloxPlayerLauncher.exe\n";

    #[test]
    fn parses_sample_manifest() {
        let pkgs = parse_package_manifest(SAMPLE).expect("parse");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "RobloxApp.zip");
        assert_eq!(pkgs[0].packed_size, 100);
        assert_eq!(pkgs[0].size, 200);
        assert_eq!(pkgs[1].signature, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    }

    #[test]
    fn rejects_bad_versions() {
        assert!(parse_package_manifest("").is_err());
        assert!(parse_package_manifest("v9\n").is_err());
        assert!(parse_package_manifest("v0\n").is_err());
    }
}
