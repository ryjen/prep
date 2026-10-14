//! Safely write a previously locked, immutable USTAR snapshot beneath a
//! directory capability. This has no path-based ambient filesystem authority.
//! Callers must supply a private store-owned target and discard it on failure;
//! no source tree is published or made eligible for builds by this function.
use crate::{
    ArchiveEntryKind, ArchiveLimits, SourceVerificationError, VerifiedUstarSnapshot,
    validate_archive_inventory,
};
use std::fmt;
use std::fs::File;
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

#[cfg(unix)]
impl From<rustix::io::Errno> for SourceExtractionError {
    fn from(value: rustix::io::Errno) -> Self {
        Self::Io(io::Error::from_raw_os_error(value.raw_os_error()))
    }
}

#[cfg(unix)]
fn directory_is_empty(directory: &File) -> Result<bool, SourceExtractionError> {
    let mut entries = rustix::fs::Dir::read_from(directory)?;
    for item in &mut entries {
        let entry = item?;
        if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(unix)]
fn ensure_subdirectory(
    parent: &rustix::fd::OwnedFd,
    component: &str,
    relative_path: &str,
    created: &mut std::collections::BTreeSet<String>,
) -> Result<rustix::fd::OwnedFd, SourceExtractionError> {
    use rustix::fs::{Mode, OFlags, mkdirat, openat};
    match mkdirat(parent, component, Mode::from_bits_truncate(0o700)) {
        Ok(()) => {
            created.insert(relative_path.to_owned());
        }
        Err(rustix::io::Errno::EXIST) if created.contains(relative_path) => {}
        Err(rustix::io::Errno::EXIST) => return Err(SourceExtractionError::UnsafeTarget),
        Err(error) => return Err(error.into()),
    }
    Ok(openat(
        parent,
        component,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?)
}

/// Only a trusted private Unix directory FD is accepted. Every untrusted
/// component is opened relative to an already-held directory descriptor with
/// NOFOLLOW; no source tree is published or made eligible for builds.
#[cfg(unix)]
pub fn extract_verified_ustar(
    snapshot: &VerifiedUstarSnapshot,
    destination: &File,
    limits: ArchiveLimits,
) -> Result<u64, SourceExtractionError> {
    use rustix::fs::{Mode, OFlags, openat};
    let expected = validate_archive_inventory(snapshot.inventory(), limits)?;
    if !directory_is_empty(destination)? {
        return Err(SourceExtractionError::UnsafeTarget);
    }
    let mut created = std::collections::BTreeSet::new();
    let mut written = 0u64;
    for entry in snapshot.inventory() {
        let mut parent = openat(
            destination,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let mut components = entry.name.split('/').peekable();
        let mut relative_path = String::new();
        while let Some(component) = components.next() {
            if !relative_path.is_empty() {
                relative_path.push('/');
            }
            relative_path.push_str(component);
            if components.peek().is_some() || entry.kind == ArchiveEntryKind::Directory {
                parent = ensure_subdirectory(&parent, component, &relative_path, &mut created)?;
                continue;
            }
            if entry.kind != ArchiveEntryKind::RegularFile {
                return Err(SourceVerificationError::UnsafeEntryPath.into());
            }
            let bytes = snapshot
                .file_bytes(&entry.name)
                .ok_or(SourceVerificationError::UnsafeEntryPath)?;
            if bytes.len() as u64 != entry.expanded_bytes {
                return Err(SourceVerificationError::IdentityMismatch.into());
            }
            written = written
                .checked_add(bytes.len() as u64)
                .ok_or(SourceVerificationError::TooLarge)?;
            if written > limits.max_total_bytes || bytes.len() as u64 > limits.max_entry_bytes {
                return Err(SourceVerificationError::TooLarge.into());
            }
            let fd = openat(
                &parent,
                component,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_bits_truncate(0o600),
            )?;
            let mut file = File::from(fd);
            file.write_all(bytes)?;
            file.sync_all()?;
        }
    }
    if written != expected {
        return Err(SourceVerificationError::IdentityMismatch.into());
    }
    Ok(written)
}

#[cfg(not(unix))]
pub fn extract_verified_ustar(
    _snapshot: &VerifiedUstarSnapshot,
    _destination: &File,
    _limits: ArchiveLimits,
) -> Result<u64, SourceExtractionError> {
    Err(SourceExtractionError::UnsafeTarget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_verified_ustar_snapshot;
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
        fn capability(&self) -> File {
            File::open(&self.0).unwrap()
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
