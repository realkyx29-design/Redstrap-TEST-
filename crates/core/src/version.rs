//! Version string parsing and comparison.
//!
//! Roblox and Red Strap both use dotted numeric versions (`0.636.0.58101`,
//! `v1.2.3`), optionally with SemVer-style pre-release (`-beta.1`) or build
//! (`+abc123`) suffixes. Comparison is purely numeric per component, which
//! matches the behaviour of the original `System.Version`-based comparer.

use std::cmp::Ordering;

/// Parsed numeric version: up to four components, missing parts are zero.
pub type Parsed = [u64; 4];

/// Parse a dotted numeric version, tolerating a leading `v` and any
/// `-suffix` / `+metadata` trailers. Returns `None` when no numeric
/// component could be parsed.
pub fn parse(input: &str) -> Option<Parsed> {
    let mut text = input.trim();
    if let Some(rest) = text.strip_prefix('v').or_else(|| text.strip_prefix('V')) {
        text = rest;
    }
    // Strip build metadata and pre-release trailers.
    if let Some(idx) = text.find('+') {
        text = &text[..idx];
    }
    if let Some(idx) = text.find('-') {
        text = &text[..idx];
    }

    let mut out = [0u64; 4];
    let mut any = false;
    for (slot, part) in out.iter_mut().zip(text.split('.')) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.parse::<u64>() {
            Ok(n) => {
                *slot = n;
                any = true;
            }
            Err(_) => return None,
        }
    }

    if any {
        Some(out)
    } else {
        None
    }
}

/// Compare two version strings numerically.
///
/// Unparseable input sorts below any parseable version (and equal to other
/// unparseable input) so update checks degrade gracefully instead of
/// erroring out on unexpected tags.
pub fn compare(left: &str, right: &str) -> Ordering {
    match (parse(left), parse(right)) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

/// Returns true when `candidate` is strictly newer than `current`.
pub fn is_newer(current: &str, candidate: &str) -> bool {
    compare(current, candidate) == Ordering::Less
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_shapes() {
        assert_eq!(parse("1.2.3"), Some([1, 2, 3, 0]));
        assert_eq!(parse("v2.0"), Some([2, 0, 0, 0]));
        assert_eq!(parse("0.636.0.58101"), Some([0, 636, 0, 58101]));
        assert_eq!(parse("1.4.0-beta.2"), Some([1, 4, 0, 0]));
        assert_eq!(parse("1.4.0+20240101"), Some([1, 4, 0, 0]));
        assert_eq!(parse("  3 "), Some([3, 0, 0, 0]));
        assert_eq!(parse(""), None);
        assert_eq!(parse("abc"), None);
        assert_eq!(parse("1.x"), None);
    }

    #[test]
    fn compares_numerically() {
        assert_eq!(compare("1.2.3", "1.2.3"), Ordering::Equal);
        assert_eq!(compare("1.2.10", "1.2.9"), Ordering::Greater);
        assert_eq!(compare("1.2", "1.2.0"), Ordering::Equal);
        assert_eq!(compare("0.636.0.58101", "0.636.0.58000"), Ordering::Greater);
        assert!(is_newer("v1.0.0", "v1.0.1"));
        assert!(!is_newer("1.0.1", "1.0.0"));
    }
}
