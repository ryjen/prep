//! Archive extraction admission policy. This is a pure preflight boundary;
//! an extractor must call it before any filesystem writes and must separately
//! enforce descriptor-relative path containment.
use crate::source::{SourceVerificationError, validate_archive_entry_path};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveEntryKind {
    RegularFile,
    Directory,
    Symlink,
    Hardlink,
    Special,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    pub name: String,
    pub kind: ArchiveEntryKind,
    pub expanded_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct ArchiveLimits {
    pub max_entries: usize,
    pub max_total_bytes: u64,
    pub max_entry_bytes: u64,
    pub max_path_bytes: usize,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_total_bytes: 1_073_741_824,
            max_entry_bytes: 268_435_456,
            max_path_bytes: 4096,
        }
    }
}

/// Validate a complete, independently parsed archive entry inventory.
/// The caller must still enforce streaming limits while extracting; metadata
/// can lie about the actual expanded payload.
pub fn validate_archive_inventory(
    entries: &[ArchiveEntry],
    limits: ArchiveLimits,
) -> Result<u64, SourceVerificationError> {
    if entries.len() > limits.max_entries {
        return Err(SourceVerificationError::TooLarge);
    }
    let mut total = 0_u64;
    let mut paths = BTreeMap::new();
    for entry in entries {
        validate_archive_entry_path(&entry.name, limits.max_path_bytes)?;
        // The v1 portable inventory admits only ASCII names; a later source
        // provider can explicitly support Unicode filesystem normalization.
        if !entry.name.is_ascii()
            || entry.name.split('/').any(|component| {
                component.ends_with('.')
                    || component.ends_with(' ')
                    || windows_reserved_name(component)
            })
        {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        if !matches!(
            entry.kind,
            ArchiveEntryKind::RegularFile | ArchiveEntryKind::Directory
        ) {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        if paths
            .insert(entry.name.to_ascii_lowercase(), entry.kind)
            .is_some()
        {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        if entry.kind == ArchiveEntryKind::Directory && entry.expanded_bytes != 0 {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        if entry.expanded_bytes > limits.max_entry_bytes {
            return Err(SourceVerificationError::TooLarge);
        }
        total = total
            .checked_add(entry.expanded_bytes)
            .ok_or(SourceVerificationError::TooLarge)?;
        if total > limits.max_total_bytes {
            return Err(SourceVerificationError::TooLarge);
        }
    }
    for path in paths.keys() {
        for (index, byte) in path.bytes().enumerate() {
            if byte == b'/'
                && matches!(
                    paths.get(&path[..index]),
                    Some(kind) if *kind != ArchiveEntryKind::Directory
                )
            {
                return Err(SourceVerificationError::UnsafeEntryPath);
            }
        }
    }
    Ok(total)
}

fn windows_reserved_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    let name = stem.to_ascii_uppercase();
    matches!(
        name.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (name.len() == 4
        && (name.starts_with("COM") || name.starts_with("LPT"))
        && name.as_bytes()[3].is_ascii_digit()
        && name.as_bytes()[3] != b'0')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, bytes: u64) -> ArchiveEntry {
        ArchiveEntry {
            name: name.into(),
            kind: ArchiveEntryKind::RegularFile,
            expanded_bytes: bytes,
        }
    }

    #[test]
    fn valid_inventory_accounts_for_expanded_content() {
        let entries = [
            ArchiveEntry {
                name: "src".into(),
                kind: ArchiveEntryKind::Directory,
                expanded_bytes: 0,
            },
            file("src/main.c", 12),
        ];
        assert_eq!(
            validate_archive_inventory(&entries, ArchiveLimits::default()).unwrap(),
            12
        );
    }

    #[test]
    fn entry_types_duplicates_and_collisions_fail_closed() {
        let limits = ArchiveLimits::default();
        assert!(validate_archive_inventory(&[file("x", 1), file("x", 1)], limits).is_err());
        assert!(validate_archive_inventory(&[file("src", 1), file("src/a", 1)], limits).is_err());
        assert!(
            validate_archive_inventory(&[file("README", 1), file("readme", 1)], limits).is_err()
        );
        assert!(validate_archive_inventory(&[file("SRC", 1), file("src/a", 1)], limits).is_err());
        assert!(validate_archive_inventory(&[file("café.txt", 1)], limits).is_err());
        for alias in [
            "foo.", "dir/bar ", "a/...", "NUL", "nul.txt", "CON", "prn.log", "COM1", "lpt9.txt",
            "AUX", "CONIN$", "CONOUT$",
        ] {
            assert!(
                validate_archive_inventory(&[file(alias, 1)], limits).is_err(),
                "accepted nonportable archive member: {alias}"
            );
        }

        for kind in [
            ArchiveEntryKind::Symlink,
            ArchiveEntryKind::Hardlink,
            ArchiveEntryKind::Special,
        ] {
            let entry = ArchiveEntry {
                name: "x".into(),
                kind,
                expanded_bytes: 0,
            };
            assert!(validate_archive_inventory(&[entry], limits).is_err());
        }
    }

    #[test]
    fn entry_and_total_limits_are_independent() {
        let limits = ArchiveLimits {
            max_entries: 1,
            max_total_bytes: 10,
            max_entry_bytes: 8,
            max_path_bytes: 5,
        };
        assert!(validate_archive_inventory(&[file("a", 1), file("b", 1)], limits).is_err());
        assert!(validate_archive_inventory(&[file("a", 9)], limits).is_err());
        assert!(validate_archive_inventory(&[file("longpath", 1)], limits).is_err());
        let limits = ArchiveLimits {
            max_entries: 3,
            max_total_bytes: 10,
            max_entry_bytes: 8,
            max_path_bytes: 20,
        };
        assert!(validate_archive_inventory(&[file("a", 6), file("b", 6)], limits).is_err());
    }
}
