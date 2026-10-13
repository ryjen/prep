//! Connect private source staging and offline cache to trusted-core lock verification.
//! The returned representation owns verified bytes but does not publish files to builders.
use crate::{
    ArchiveLimits, SourceVerificationError, VerifiedUstarSnapshot, read_verified_ustar_snapshot,
};
use prep_manifest::LockedSource;
use prep_store::{Store, StoreError, UnverifiedSourceStage};
use std::fmt;
use std::io::Cursor;

#[derive(Debug)]
pub enum SourceLoadError {
    Store(StoreError),
    Verification(SourceVerificationError),
}
impl fmt::Display for SourceLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "source store: {error}"),
            Self::Verification(error) => write!(formatter, "source verification: {error}"),
        }
    }
}
impl std::error::Error for SourceLoadError {}
impl From<StoreError> for SourceLoadError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}
impl From<SourceVerificationError> for SourceLoadError {
    fn from(error: SourceVerificationError) -> Self {
        Self::Verification(error)
    }
}

/// Ingest staged but unverified archive bytes into an independently locked,
/// bounded in-memory snapshot. Failure leaves staging untrusted and unexposed.
pub fn load_staged_ustar(
    stage: &UnverifiedSourceStage,
    lock: &LockedSource,
    limits: ArchiveLimits,
    maximum: u64,
) -> Result<VerifiedUstarSnapshot, SourceLoadError> {
    let mut file = stage.open_unverified_archive()?;
    Ok(read_verified_ustar_snapshot(
        lock, &mut file, limits, maximum,
    )?)
}

/// Offline cache hit is never assumed trustworthy: Store rehashes its blob and
/// core independently validates the locked digest and archive semantics.
pub fn load_cached_ustar(
    store: &Store,
    lock: &LockedSource,
    limits: ArchiveLimits,
    maximum: u64,
) -> Result<Option<VerifiedUstarSnapshot>, SourceLoadError> {
    let digest = match lock {
        LockedSource::Archive { sha256, .. } => sha256.as_str(),
        LockedSource::Git { .. } => return Err(SourceVerificationError::WrongKind.into()),
        LockedSource::Path { .. } => return Err(SourceVerificationError::NonImmutable.into()),
    };
    let Some(bytes) = store.read_cached_source_blob(digest, maximum)? else {
        return Ok(None);
    };
    Ok(Some(read_verified_ustar_snapshot(
        lock,
        &mut Cursor::new(bytes),
        limits,
        maximum,
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prep_manifest::{PackageVersion, PluginName, ProviderIdentity, Sha256Digest, SourceUrl};
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct TestRoot(std::path::PathBuf);
    impl TestRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "prep-source-load-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("unique test root");
            Self(path)
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn tar() -> Vec<u8> {
        let mut header = [0u8; 512];
        header[0..10].copy_from_slice(b"readme.txt");
        header[124..136].copy_from_slice(b"00000000005\0");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: u64 = header
            .iter()
            .enumerate()
            .map(|(i, b)| u64::from(if (148..156).contains(&i) { b' ' } else { *b }))
            .sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        let mut archive = header.to_vec();
        archive.extend_from_slice(b"hello");
        archive.resize(1024, 0);
        archive.extend_from_slice(&[0; 1024]);
        archive
    }

    fn lock(bytes: &[u8]) -> LockedSource {
        LockedSource::Archive {
            url: SourceUrl::parse("https://example.test/readme.tar").unwrap(),
            sha256: Sha256Digest::parse(format!("{:x}", Sha256::digest(bytes))).unwrap(),
            provider: ProviderIdentity {
                name: PluginName::parse("archive").unwrap(),
                version: PackageVersion::parse("1.0.0").unwrap(),
            },
        }
    }

    #[test]
    fn staged_then_offline_cached_archive_remains_lock_verified() {
        let temp = TestRoot::new();
        let store = Store::open(&temp.0.join("store")).unwrap();
        let stage = store.begin_unverified_source().unwrap();
        let bytes = tar();
        let source = lock(&bytes);
        let digest = match &source {
            LockedSource::Archive { sha256, .. } => sha256.as_str(),
            _ => unreachable!(),
        };
        stage
            .spool_unverified_archive(&mut Cursor::new(&bytes), bytes.len() as u64)
            .unwrap();
        let verified = load_staged_ustar(
            &stage,
            &source,
            ArchiveLimits::default(),
            bytes.len() as u64,
        )
        .unwrap();
        assert_eq!(verified.file_bytes("readme.txt"), Some(&b"hello"[..]));
        store
            .cache_source_blob(&stage, digest, bytes.len() as u64)
            .unwrap();
        drop(stage);
        let offline = load_cached_ustar(
            &store,
            &source,
            ArchiveLimits::default(),
            bytes.len() as u64,
        )
        .unwrap()
        .unwrap();
        assert_eq!(offline.file_bytes("readme.txt"), Some(&b"hello"[..]));
        let cache_path = store.root().join("sources/sha256").join(digest);
        fs::write(cache_path, b"altered").unwrap();
        assert!(matches!(
            load_cached_ustar(
                &store,
                &source,
                ArchiveLimits::default(),
                bytes.len() as u64
            ),
            Err(SourceLoadError::Store(StoreError::CorruptSource { .. }))
        ));
    }

    #[test]
    fn missing_archive_offline_is_not_a_verified_source() {
        let temp = TestRoot::new();
        let store = Store::open(&temp.0.join("store")).unwrap();
        let sample = tar();
        assert!(
            load_cached_ustar(
                &store,
                &lock(&sample),
                ArchiveLimits::default(),
                sample.len() as u64
            )
            .unwrap()
            .is_none()
        );
    }
}
