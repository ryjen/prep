//! Strict preflight for NUL-delimited `git ls-tree -rz --full-tree` output.
//! Must be run on the pinned commit before any worktree checkout. Object
//! identity and checkout integrity still require independent verification.
use crate::source::{SourceVerificationError, validate_archive_entry_path};
use std::collections::BTreeSet;

pub fn validate_git_tree_listing(
    bytes: &[u8],
    max_entries: usize,
    max_path_bytes: usize,
) -> Result<usize, SourceVerificationError> {
    if bytes.is_empty() {
        return Ok(0);
    }
    if !bytes.ends_with(&[0]) {
        return Err(SourceVerificationError::UnsafeEntryPath);
    }
    let mut entries = BTreeSet::new();
    let mut expected_object_id_length = None;
    let mut count = 0;
    for record in bytes[..bytes.len() - 1].split(|byte| *byte == 0) {
        count += 1;
        if count > max_entries {
            return Err(SourceVerificationError::TooLarge);
        }
        let (header, raw_path) = record
            .split_once_byte(b'\t')
            .ok_or(SourceVerificationError::UnsafeEntryPath)?;
        let path =
            std::str::from_utf8(raw_path).map_err(|_| SourceVerificationError::UnsafeEntryPath)?;
        validate_archive_entry_path(path, max_path_bytes)?;
        if !entries.insert(path) {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        let mut parts = header.split(|byte| *byte == b' ');
        let mode = parts
            .next()
            .ok_or(SourceVerificationError::UnsafeEntryPath)?;
        let kind = parts
            .next()
            .ok_or(SourceVerificationError::UnsafeEntryPath)?;
        let object_id = parts
            .next()
            .ok_or(SourceVerificationError::UnsafeEntryPath)?;
        if parts.next().is_some() || !matches!(mode, b"100644" | b"100755") || kind != b"blob" {
            // Reject gitlinks (160000), symlinks (120000), and unknown modes.
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        if !matches!(object_id.len(), 40 | 64) || !object_id.iter().all(u8::is_ascii_hexdigit) {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        if expected_object_id_length.is_some_and(|length| length != object_id.len()) {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        expected_object_id_length = Some(object_id.len());
    }
    Ok(count)
}

trait SplitOnceByte {
    fn split_once_byte(&self, value: u8) -> Option<(&[u8], &[u8])>;
}

impl SplitOnceByte for [u8] {
    fn split_once_byte(&self, value: u8) -> Option<(&[u8], &[u8])> {
        let index = self.iter().position(|byte| *byte == value)?;
        Some((&self[..index], &self[index + 1..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(mode: &str, kind: &str, object_id: &str, path: &str) -> Vec<u8> {
        format!("{mode} {kind} {object_id}\t{path}\0").into_bytes()
    }

    #[test]
    fn rejects_gitlink_symlink_traversal_and_bad_object_id() {
        let id = "a".repeat(40);
        assert_eq!(
            validate_git_tree_listing(&entry("100644", "blob", &id, "src/main.c"), 5, 256).unwrap(),
            1
        );
        for (mode, kind, path) in [
            ("160000", "commit", "lib"),
            ("120000", "blob", "link"),
            ("100644", "blob", "../escape"),
            ("100644", "blob", "/root/escape"),
            ("100644", "blob", "src\\escape"),
            ("040000", "tree", "src"),
        ] {
            assert!(validate_git_tree_listing(&entry(mode, kind, &id, path), 5, 256).is_err());
        }
        assert!(
            validate_git_tree_listing(&entry("100644", "blob", "not-a-sha", "x"), 5, 256).is_err()
        );
    }

    #[test]
    fn rejects_truncated_duplicate_and_excess_entries() {
        let id = "a".repeat(40);
        let valid = entry("100644", "blob", &id, "a");
        assert!(validate_git_tree_listing(&valid[..valid.len() - 1], 5, 256).is_err());
        let duplicate = [valid.clone(), valid.clone()].concat();
        assert!(validate_git_tree_listing(&duplicate, 5, 256).is_err());
        let distinct = [valid, entry("100755", "blob", &id, "b")].concat();
        assert_eq!(validate_git_tree_listing(&distinct, 2, 256).unwrap(), 2);
        assert!(validate_git_tree_listing(&distinct, 1, 256).is_err());
    }
}
