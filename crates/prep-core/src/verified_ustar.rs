//! Small immutable USTAR snapshots: locked digest -> validated members -> owned bytes.
//! This deliberately performs no filesystem extraction or source publication.
use crate::{
    ArchiveEntry, ArchiveEntryKind, ArchiveLimits, SourceVerificationError, inspect_ustar,
    verify_archive_bytes,
};
use prep_manifest::{LockedSource, Sha256Digest};
use std::collections::BTreeMap;
use std::io::{Cursor, Read};

/// A conservative cap on in-memory source snapshots. Larger sources must use
/// a separate verified, streamed and descriptor-relative materializer.
pub const MAX_ARCHIVE_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

/// Fully owned bytes with SHA-256 integrity and USTAR inventory already checked.
/// No mutable reader or filesystem path escapes this type.
pub struct VerifiedUstarSnapshot {
    sha256: Sha256Digest,
    bytes: Vec<u8>,
    entries: Vec<ArchiveEntry>,
    files: BTreeMap<String, (usize, usize)>,
}

impl VerifiedUstarSnapshot {
    #[must_use]
    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }

    #[must_use]
    pub fn inventory(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    /// Obtain stable verified regular-file bytes; directories have no contents.
    #[must_use]
    pub fn file_bytes(&self, name: &str) -> Option<&[u8]> {
        let &(start, end) = self.files.get(name)?;
        self.bytes.get(start..end)
    }
}

/// Verify the *entire* bounded input against the immutable lock before parsing.
/// The owned snapshot prevents a changing source FD from invalidating the result.
/// Does not grant permission to publish source trees or run build plugins.
pub fn read_verified_ustar_snapshot(
    source: &LockedSource,
    input: &mut impl Read,
    limits: ArchiveLimits,
    max_archive_bytes: u64,
) -> Result<VerifiedUstarSnapshot, SourceVerificationError> {
    let sha256 = match source {
        LockedSource::Archive { sha256, .. } => sha256.clone(),
        LockedSource::Path { .. } => return Err(SourceVerificationError::NonImmutable),
        LockedSource::Git { .. } => return Err(SourceVerificationError::WrongKind),
    };
    if max_archive_bytes > MAX_ARCHIVE_SNAPSHOT_BYTES {
        return Err(SourceVerificationError::TooLarge);
    }
    let mut bytes = Vec::new();
    input
        .take(max_archive_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(SourceVerificationError::Io)?;
    if bytes.len() as u64 > max_archive_bytes {
        return Err(SourceVerificationError::TooLarge);
    }
    verify_archive_bytes(source, &mut Cursor::new(&bytes), max_archive_bytes)?;
    let entries = inspect_ustar(&mut Cursor::new(&bytes), limits, max_archive_bytes)?;
    let mut files = BTreeMap::new();
    let mut offset = 0usize;
    for entry in &entries {
        let start = offset
            .checked_add(512)
            .ok_or(SourceVerificationError::TooLarge)?;
        let size =
            usize::try_from(entry.expanded_bytes).map_err(|_| SourceVerificationError::TooLarge)?;
        let end = start
            .checked_add(size)
            .ok_or(SourceVerificationError::TooLarge)?;
        let padded = size
            .checked_add(511)
            .ok_or(SourceVerificationError::TooLarge)?
            / 512;
        offset = start
            .checked_add(
                padded
                    .checked_mul(512)
                    .ok_or(SourceVerificationError::TooLarge)?,
            )
            .ok_or(SourceVerificationError::TooLarge)?;
        if offset > bytes.len() || end > bytes.len() {
            return Err(SourceVerificationError::UnsafeEntryPath);
        }
        if entry.kind == ArchiveEntryKind::RegularFile {
            files.insert(entry.name.clone(), (start, end));
        }
    }
    Ok(VerifiedUstarSnapshot {
        sha256,
        bytes,
        entries,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prep_manifest::{PackageVersion, PluginName, ProviderIdentity, SourceUrl};
    use sha2::{Digest, Sha256};

    fn make_archive(name: &str, data: &[u8], kind: u8) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        header[156] = kind;
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let checksum: u64 = header
            .iter()
            .enumerate()
            .map(|(i, b)| u64::from(if (148..156).contains(&i) { b' ' } else { *b }))
            .sum();
        header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(data);
        bytes.resize(512 + data.len().div_ceil(512) * 512, 0);
        bytes.extend_from_slice(&[0; 1024]);
        bytes
    }

    fn locked(bytes: &[u8]) -> LockedSource {
        LockedSource::Archive {
            url: SourceUrl::parse("https://example.test/source.tar").unwrap(),
            sha256: Sha256Digest::parse(format!("{:x}", Sha256::digest(bytes))).unwrap(),
            provider: ProviderIdentity {
                name: PluginName::parse("archive").unwrap(),
                version: PackageVersion::parse("1.0.0").unwrap(),
            },
        }
    }

    #[test]
    fn exposes_only_stable_bytes_from_validated_locked_archive() {
        let archive = make_archive("src/main.rs", b"fn main() {}\n", b'0');
        let source = locked(&archive);
        let snapshot = read_verified_ustar_snapshot(
            &source,
            &mut Cursor::new(&archive),
            ArchiveLimits::default(),
            archive.len() as u64,
        )
        .unwrap();
        assert_eq!(snapshot.inventory().len(), 1);
        assert_eq!(
            snapshot.file_bytes("src/main.rs"),
            Some(&b"fn main() {}\n"[..])
        );
        assert_eq!(snapshot.file_bytes("src/other.rs"), None);
        let mut altered = archive.clone();
        altered[520] ^= 1;
        assert!(matches!(
            read_verified_ustar_snapshot(
                &source,
                &mut Cursor::new(altered),
                ArchiveLimits::default(),
                4096
            ),
            Err(SourceVerificationError::IdentityMismatch)
        ));
    }

    #[test]
    fn never_admits_wrong_digest_size_or_unsupported_member_types() {
        let archive = make_archive("link", b"", b'2');
        let source = locked(&archive);
        assert!(
            read_verified_ustar_snapshot(
                &source,
                &mut Cursor::new(&archive),
                ArchiveLimits::default(),
                4096
            )
            .is_err()
        );
        assert!(matches!(
            read_verified_ustar_snapshot(
                &source,
                &mut Cursor::new(&archive),
                ArchiveLimits::default(),
                64
            ),
            Err(SourceVerificationError::TooLarge)
        ));
        assert!(matches!(
            read_verified_ustar_snapshot(
                &source,
                &mut Cursor::new(&archive),
                ArchiveLimits::default(),
                MAX_ARCHIVE_SNAPSHOT_BYTES + 1
            ),
            Err(SourceVerificationError::TooLarge)
        ));
    }
}
