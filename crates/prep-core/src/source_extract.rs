//! Safely write a previously locked, immutable USTAR snapshot beneath a
//! directory capability. This has no path-based ambient filesystem authority.
//! Callers must supply a private store-owned target and discard it on failure;
//! no source tree is published or made eligible for builds by this function.
use crate::{
    ArchiveEntryKind, ArchiveLimits, SourceVerificationError, VerifiedUstarSnapshot,
    validate_archive_inventory,
};
use cap_std::fs::{Dir, OpenOptions};
use std::fmt;
use std::io::{self, Write};

#[derive(Debug)]
pub enum SourceExtractionError {
    InvalidSource(SourceVerificationError),
    UnsafeTarget,
    Io(io::Error),
}
impl fmt::Display for SourceExtractionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSource(error) => write!(f, "invalid source archive: {error}"),
            Self::UnsafeTarget => f.write_str("extraction directory must be empty and private"),
            Self::Io(error) => write!(f, "source extraction failed: {error}"),
        }
    }
}
impl std::error::Error for SourceExtractionError {}
impl From<io::Error> for SourceExtractionError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<SourceVerificationError> for SourceExtractionError {
    fn from(value: SourceVerificationError) -> Self {
        Self::InvalidSource(value)
    }
}

fn ensure_real_directory(parent: &Dir, name: &str) -> Result<Dir, SourceExtractionError> {
    match parent.create_dir(name) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = parent.symlink_metadata(name)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(SourceExtractionError::UnsafeTarget);
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(parent.open_dir(name)?)
}

/// Extracts into an empty directory handle using only capability-relative
/// operations. Files use create_new; no overwrites, links, or special files.
/// The only data source is a digest-checked, owned immutable USTAR snapshot.
/// On error, discard the whole private target; this function does not publish.
pub fn extract_verified_ustar(
    snapshot: &VerifiedUstarSnapshot,
    destination: &Dir,
    limits: ArchiveLimits,
) -> Result<u64, SourceExtractionError> {
    let expected = validate_archive_inventory(snapshot.inventory(), limits)?;
    if destination.read_dir(".")?.next().is_some() {
        return Err(SourceExtractionError::UnsafeTarget);
    }
    let mut written = 0_u64;
    for entry in snapshot.inventory() {
        let mut parent = destination.try_clone()?;
        let mut components = entry.name.split('/').peekable();
        while let Some(component) = components.next() {
            if components.peek().is_some() || entry.kind == ArchiveEntryKind::Directory {
                parent = ensure_real_directory(&parent, component)?;
                continue;
            }
            if entry.kind != ArchiveEntryKind::RegularFile {
                return Err(SourceExtractionError::InvalidSource(
                    SourceVerificationError::UnsafeEntryPath,
                ));
            }
            let bytes =
                snapshot
                    .file_bytes(&entry.name)
                    .ok_or(SourceExtractionError::InvalidSource(
                        SourceVerificationError::UnsafeEntryPath,
                    ))?;
            if bytes.len() as u64 != entry.expanded_bytes {
                return Err(SourceExtractionError::InvalidSource(
                    SourceVerificationError::IdentityMismatch,
                ));
            }
            written = written
                .checked_add(bytes.len() as u64)
                .ok_or(SourceVerificationError::TooLarge)?;
            if written > limits.max_total_bytes {
                return Err(SourceVerificationError::TooLarge.into());
            }
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            let mut file = parent.open_with(component, &options)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
    }
    if written != expected {
        return Err(SourceVerificationError::IdentityMismatch.into());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_verified_ustar_snapshot;
    use cap_std::ambient_authority;
    use prep_manifest::{
        LockedSource, PackageVersion, PluginName, ProviderIdentity, Sha256Digest, SourceUrl,
    };
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct TemporaryDir(std::path::PathBuf);
    impl TemporaryDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "prep-archive-capability-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn capability(&self) -> Dir {
            Dir::open_ambient_dir(&self.0, ambient_authority()).unwrap()
        }
    }
    impl Drop for TemporaryDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn tar_entry(name: &str, contents: &[u8], kind: u8) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[124..136].copy_from_slice(format!("{:011o}\0", contents.len()).as_bytes());
        header[156] = kind;
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: u64 = header
            .iter()
            .enumerate()
            .map(|(i, b)| u64::from(if (148..156).contains(&i) { b' ' } else { *b }))
            .sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        let mut result = header.to_vec();
        result.extend_from_slice(contents);
        result.resize(512 + contents.len().div_ceil(512) * 512, 0);
        result
    }

    fn snapshot(tar: &[u8]) -> VerifiedUstarSnapshot {
        let source = LockedSource::Archive {
            url: SourceUrl::parse("https://example.test/source.tar").unwrap(),
            sha256: Sha256Digest::parse(format!("{:x}", Sha256::digest(tar))).unwrap(),
            provider: ProviderIdentity {
                name: PluginName::parse("archive").unwrap(),
                version: PackageVersion::parse("1.0.0").unwrap(),
            },
        };
        read_verified_ustar_snapshot(
            &source,
            &mut Cursor::new(tar),
            ArchiveLimits::default(),
            tar.len() as u64,
        )
        .unwrap()
    }

    #[test]
    fn extracts_verified_multiblock_and_directory_members() {
        let mut tar = tar_entry("src/", b"", b'5');
        let long = vec![b'Z'; 513];
        tar.extend_from_slice(&tar_entry("src/large.bin", &long, b'0'));
        tar.extend_from_slice(&tar_entry("src/end.txt", b"end", b'0'));
        tar.extend_from_slice(&[0; 1024]);
        let s = snapshot(&tar);
        let target = TemporaryDir::new();
        assert_eq!(
            extract_verified_ustar(&s, &target.capability(), ArchiveLimits::default()).unwrap(),
            516
        );
        assert_eq!(fs::read(target.0.join("src/large.bin")).unwrap(), long);
        assert_eq!(fs::read(target.0.join("src/end.txt")).unwrap(), b"end");
        assert!(
            extract_verified_ustar(&s, &target.capability(), ArchiveLimits::default()).is_err()
        );
    }

    #[test]
    fn rejects_nonempty_or_symlinked_targets_without_writing() {
        let mut tar = tar_entry("file.txt", b"safe", b'0');
        tar.extend_from_slice(&[0; 1024]);
        let s = snapshot(&tar);
        let target = TemporaryDir::new();
        fs::write(target.0.join("existing"), b"keep").unwrap();
        assert!(matches!(
            extract_verified_ustar(&s, &target.capability(), ArchiveLimits::default()),
            Err(SourceExtractionError::UnsafeTarget)
        ));
        assert!(!target.0.join("file.txt").exists());
    }

    #[test]
    fn rejects_limits_before_filesystem_writes() {
        let mut tar = tar_entry("file", b"data", b'0');
        tar.extend_from_slice(&[0; 1024]);
        let s = snapshot(&tar);
        let target = TemporaryDir::new();
        let limits = ArchiveLimits {
            max_total_bytes: 2,
            ..ArchiveLimits::default()
        };
        assert!(extract_verified_ustar(&s, &target.capability(), limits).is_err());
        assert!(fs::read_dir(&target.0).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn existing_symlink_is_never_used_as_a_target_directory() {
        use std::os::unix::fs::symlink;
        let mut tar = tar_entry("src/file", b"data", b'0');
        tar.extend_from_slice(&[0; 1024]);
        let s = snapshot(&tar);
        let target = TemporaryDir::new();
        let outside = TemporaryDir::new();
        symlink(&outside.0, target.0.join("src")).unwrap();
        assert!(
            extract_verified_ustar(&s, &target.capability(), ArchiveLimits::default()).is_err()
        );
        assert!(!outside.0.join("file").exists());
    }
}
