use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const RESULT_SCHEMA_V1: &str = "prep.result/1";
const RESULT_RETENTION_PINNED: &str = "pinned";
const MAX_CACHE_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
static TRANSACTION_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub enum StoreError {
    InvalidPath(String),
    InvalidResultId(String),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Serialization(String),
    CorruptResult {
        path: PathBuf,
        message: String,
    },
    InvalidOutput {
        path: PathBuf,
        message: String,
    },
    CrossFilesystem,
    SourceTooLarge,
    CorruptSource {
        path: PathBuf,
    },
    Activation(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(message) => formatter.write_str(message),
            Self::InvalidResultId(message) => formatter.write_str(message),
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {}: {source}", path.display()),
            Self::Serialization(message) => formatter.write_str(message),
            Self::CorruptResult { path, message } => {
                write!(formatter, "corrupt result {}: {message}", path.display())
            }
            Self::InvalidOutput { path, message } => {
                write!(formatter, "invalid output {}: {message}", path.display())
            }
            Self::SourceTooLarge => {
                formatter.write_str("unverified source input exceeds size limit")
            }
            Self::CorruptSource { path } => {
                write!(formatter, "source digest mismatch: {}", path.display())
            }
            Self::CrossFilesystem => write!(
                formatter,
                "result staging and destination are on different filesystems"
            ),
            Self::Activation(message) => formatter.write_str(message),
        }
    }
}

impl Error for StoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreResultId {
    hex: String,
}

impl StoreResultId {
    pub fn parse(value: impl AsRef<str>) -> Result<Self, StoreError> {
        let value = value.as_ref();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(StoreError::InvalidResultId(
                "result identity must use sha256:<64 hex> form".to_owned(),
            ));
        };
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(StoreError::InvalidResultId(
                "result identity must contain exactly 64 hexadecimal characters".to_owned(),
            ));
        }
        Ok(Self {
            hex: hex.to_ascii_lowercase(),
        })
    }

    #[must_use]
    pub fn as_string(&self) -> String {
        format!("sha256:{}", self.hex)
    }

    #[must_use]
    pub fn hex(&self) -> &str {
        &self.hex
    }
}

impl fmt::Display for StoreResultId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPaths {
    pub data_root: PathBuf,
    pub cache_root: PathBuf,
    pub store_root: PathBuf,
}

impl UserPaths {
    pub fn resolve(
        home: &Path,
        xdg_data_home: Option<&Path>,
        xdg_cache_home: Option<&Path>,
    ) -> Result<Self, StoreError> {
        require_absolute("home", home)?;
        if let Some(path) = xdg_data_home {
            require_absolute("XDG_DATA_HOME", path)?;
        }
        if let Some(path) = xdg_cache_home {
            require_absolute("XDG_CACHE_HOME", path)?;
        }

        let data_root = xdg_data_home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("prep");
        let cache_root = xdg_cache_home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".cache"))
            .join("prep");
        let store_root = data_root.join("store");

        Ok(Self {
            data_root,
            cache_root,
            store_root,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPaths {
    pub project_root: PathBuf,
    pub state_root: PathBuf,
}

impl ProjectPaths {
    pub fn resolve(project_root: &Path) -> Result<Self, StoreError> {
        require_absolute("project root", project_root)?;
        Ok(Self {
            project_root: project_root.to_path_buf(),
            state_root: project_root.join(".prep"),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    staging_root: PathBuf,
    source_staging_root: PathBuf,
    source_cache_root: PathBuf,
    lock_root: PathBuf,
    results_sha256_root: PathBuf,
    store_lock_path: PathBuf,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        require_absolute("store root", root)?;
        fs::create_dir_all(root).map_err(|source| io_error("create store root", root, source))?;
        let root = fs::canonicalize(root)
            .map_err(|source| io_error("canonicalize store root", root, source))?;
        require_real_directory(&root, "store root")?;

        let staging_root = ensure_child_directory(&root, ".staging")?;
        let source_staging_root = ensure_child_directory(&root, ".source-staging")?;
        let sources = ensure_child_directory(&root, "sources")?;
        let source_cache_root = ensure_child_directory(&sources, "sha256")?;
        let lock_root = ensure_child_directory(&root, ".locks")?;
        let results_root = ensure_child_directory(&root, "results")?;
        let results_sha256_root = ensure_child_directory(&results_root, "sha256")?;
        let store_lock_path = root.join(".store.lock");
        let _ = open_lock_file(&store_lock_path)?;

        Ok(Self {
            root,
            staging_root,
            source_staging_root,
            source_cache_root,
            lock_root,
            results_sha256_root,
            store_lock_path,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn begin(&self) -> Result<StoreTransaction, StoreError> {
        let _store_lock = self.lock_store()?;

        for _ in 0..128 {
            let id = transaction_id();
            let lock_path = self.lock_root.join(format!("{id}.lock"));
            let lease = match create_new_lock_file(&lock_path) {
                Ok(file) => file,
                Err(StoreError::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::AlreadyExists =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            };
            lease
                .lock()
                .map_err(|source| io_error("lock staging lease", &lock_path, source))?;

            let staging_path = self.staging_root.join(&id);
            match fs::create_dir(&staging_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = release_and_remove_lock_file(lease, &lock_path);
                    continue;
                }
                Err(source) => {
                    let _ = release_and_remove_lock_file(lease, &lock_path);
                    return Err(io_error("create staging directory", &staging_path, source));
                }
            }

            let prefix = staging_path.join("prefix");
            if let Err(source) = fs::create_dir(&prefix) {
                let _ = fs::remove_dir_all(&staging_path);
                let _ = release_and_remove_lock_file(lease, &lock_path);
                return Err(io_error("create staging prefix", &prefix, source));
            }

            return Ok(StoreTransaction {
                store: self.clone(),
                staging_path,
                prefix,
                lock_path,
                lease: Some(lease),
                finished: false,
            });
        }

        Err(StoreError::InvalidPath(
            "unable to allocate a unique staging transaction".to_owned(),
        ))
    }

    /// Allocate an unverified source workspace isolated from build-result staging.
    /// Source bytes placed here cannot be published using `StoreTransaction::commit`.
    /// The workspace is deleted on drop; it does not imply source verification.
    pub fn begin_unverified_source(&self) -> Result<UnverifiedSourceStage, StoreError> {
        let _store_lock = self.lock_store()?;
        for _ in 0..128 {
            let id = transaction_id();
            let lock_path = self.lock_root.join(format!("{id}.lock"));
            let lease = match create_new_lock_file(&lock_path) {
                Ok(file) => file,
                Err(StoreError::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::AlreadyExists =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            };
            lease
                .lock()
                .map_err(|source| io_error("lock source staging lease", &lock_path, source))?;
            let path = self.source_staging_root.join(&id);
            #[cfg(unix)]
            let created = {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = fs::DirBuilder::new();
                builder.mode(0o700);
                builder.create(&path)
            };
            #[cfg(not(unix))]
            let created = fs::create_dir(&path);
            match created {
                Ok(()) => {
                    return Ok(UnverifiedSourceStage {
                        store: self.clone(),
                        path,
                        lock_path,
                        lease: Some(lease),
                        finished: false,
                    });
                }
                Err(source) => {
                    release_and_remove_lock_file(lease, &lock_path)?;
                    if source.kind() == std::io::ErrorKind::AlreadyExists {
                        continue;
                    }
                    return Err(io_error(
                        "create private source staging directory",
                        &path,
                        source,
                    ));
                }
            }
        }
        Err(StoreError::InvalidPath(
            "unable to allocate a private source staging transaction".to_owned(),
        ))
    }

    /// Read a previously cached source archive into independently digest-verified,
    /// immutable bytes. Returning bytes (not a path or descriptor) protects callers
    /// against cache mutation after the integrity check, including offline use.
    /// This verifies the blob digest, NOT its format or suitability for extraction.
    pub fn read_cached_source_blob(
        &self,
        expected_sha256: &str,
        max_bytes: u64,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let digest = canonical_source_sha256(expected_sha256)?;
        let _store_lock = self.lock_store()?;
        self.read_cached_source_blob_locked(&digest, max_bytes)
    }

    fn read_cached_source_blob_locked(
        &self,
        digest: &str,
        max_bytes: u64,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let path = self.source_cache_root.join(digest);
        verify_real_directory_within(&self.root, &self.source_cache_root, "source cache root")?;
        if !node_exists(&path)? {
            return Ok(None);
        }
        let mut file = File::open(&path)
            .map_err(|source| io_error("open cached source blob", &path, source))?;
        verify_open_file_identity(&path, &file)?;
        let bytes = read_bounded_source_bytes(&mut file, max_bytes)?;
        if format!("{:x}", Sha256::digest(&bytes)) != digest {
            return Err(StoreError::CorruptSource { path });
        }
        Ok(Some(bytes))
    }

    /// Cache exactly the bytes copied from a leased, unverified source stage.
    /// Verification is against the requested SHA-256 of the *copied* bytes,
    /// never a mutable path checked before the copy. Cache is still only a blob,
    /// not a source tree or permission to invoke builders.
    pub fn cache_source_blob(
        &self,
        stage: &UnverifiedSourceStage,
        expected_sha256: &str,
        max_bytes: u64,
    ) -> Result<(), StoreError> {
        let digest = canonical_source_sha256(expected_sha256)?;
        if stage.store.root != self.root {
            return Err(StoreError::InvalidPath(
                "source stage belongs to a different store".to_owned(),
            ));
        }
        let mut staged = stage.open_unverified_archive()?;
        let bytes = read_bounded_source_bytes(&mut staged, max_bytes)?;
        if format!("{:x}", Sha256::digest(&bytes)) != digest {
            return Err(StoreError::CorruptSource {
                path: stage.path.join("archive.blob"),
            });
        }

        let _store_lock = self.lock_store()?;
        verify_real_directory_within(&self.root, &self.source_cache_root, "source cache root")?;
        verify_real_directory_within(
            &self.source_staging_root,
            &stage.path,
            "leased source stage",
        )?;
        if self
            .read_cached_source_blob_locked(&digest, max_bytes)?
            .is_some()
        {
            return Ok(());
        }
        let part = stage.path.join("cache.part");
        let destination = self.source_cache_root.join(&digest);
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&part)
            .map_err(|source| io_error("create source cache part", &part, source))?;
        let write_result = (|| {
            file.write_all(&bytes)
                .map_err(|source| io_error("write source cache part", &part, source))?;
            file.sync_all()
                .map_err(|source| io_error("sync source cache part", &part, source))?;
            verify_open_file_identity(&part, &file)?;
            match fs::hard_link(&part, &destination) {
                Ok(()) => Ok(()),
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    self.read_cached_source_blob_locked(&digest, max_bytes)?
                        .ok_or(StoreError::CorruptSource {
                            path: destination.clone(),
                        })?;
                    Ok(())
                }
                Err(source) => Err(io_error(
                    "publish source blob without overwrite",
                    &destination,
                    source,
                )),
            }
        })();
        let cleanup = fs::remove_file(&part)
            .map_err(|source| io_error("remove source cache part", &part, source));
        write_result?;
        cleanup?;
        Ok(())
    }

    pub fn get(&self, id: &StoreResultId) -> Result<Option<PublishedResult>, StoreError> {
        let _store_lock = self.lock_store()?;
        let path = self.result_path(id);
        if !node_exists(&path)? {
            return Ok(None);
        }
        validate_existing_result(&path, id, &self.results_sha256_root)?;
        Ok(Some(PublishedResult {
            id: id.clone(),
            prefix: path.join("prefix"),
            path,
        }))
    }

    pub fn recover_abandoned(&self) -> Result<usize, StoreError> {
        let _store_lock = self.lock_store()?;
        let results = self.recover_staging_root(&self.staging_root)?;
        let sources = self.recover_staging_root(&self.source_staging_root)?;
        self.remove_orphaned_unlocked_leases()?;
        Ok(results + sources)
    }

    fn recover_staging_root(&self, staging_root: &Path) -> Result<usize, StoreError> {
        let mut recovered = 0;

        for entry in fs::read_dir(staging_root)
            .map_err(|source| io_error("read staging directory", staging_root, source))?
        {
            let entry =
                entry.map_err(|source| io_error("read staging entry", staging_root, source))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|source| io_error("read staging entry type", &path, source))?;
            if !file_type.is_dir() || file_type.is_symlink() {
                return Err(StoreError::InvalidOutput {
                    path,
                    message: "staging root contains a non-directory entry".to_owned(),
                });
            }

            let Some(id) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                return Err(StoreError::InvalidOutput {
                    path,
                    message: "staging transaction name is not valid UTF-8".to_owned(),
                });
            };
            if !valid_transaction_id(&id) {
                return Err(StoreError::InvalidOutput {
                    path,
                    message: "staging transaction has an invalid identifier".to_owned(),
                });
            }

            verify_real_directory_within(staging_root, &path, "staging transaction")?;
            let lock_path = self.lock_root.join(format!("{id}.lock"));
            let lease = open_existing_lock_file(&lock_path)?;

            match lease.try_lock() {
                Ok(()) => {
                    fs::remove_dir_all(&path).map_err(|source| {
                        io_error("remove abandoned staging directory", &path, source)
                    })?;
                    release_and_remove_lock_file(lease, &lock_path)?;
                    recovered += 1;
                }
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(source)) => {
                    return Err(io_error("try staging lease", &lock_path, source));
                }
            }
        }

        Ok(recovered)
    }

    fn remove_orphaned_unlocked_leases(&self) -> Result<(), StoreError> {
        for entry in fs::read_dir(&self.lock_root)
            .map_err(|source| io_error("read staging lease directory", &self.lock_root, source))?
        {
            let entry = entry
                .map_err(|source| io_error("read staging lease entry", &self.lock_root, source))?;
            let path = entry.path();
            if path.extension() != Some(OsStr::new("lock")) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(OsStr::to_str) else {
                return Err(StoreError::InvalidPath(format!(
                    "staging lease name is not valid UTF-8: {}",
                    path.display()
                )));
            };
            if !valid_transaction_id(stem) {
                return Err(StoreError::InvalidPath(format!(
                    "staging lease has an invalid transaction identifier: {}",
                    path.display()
                )));
            }
            if node_exists(&self.staging_root.join(stem))?
                || node_exists(&self.source_staging_root.join(stem))?
            {
                continue;
            }

            let lease = open_existing_lock_file(&path)?;
            match lease.try_lock() {
                Ok(()) => release_and_remove_lock_file(lease, &path)?,
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(source)) => {
                    return Err(io_error("try orphan staging lease", &path, source));
                }
            }
        }
        Ok(())
    }

    fn verify_layout(&self) -> Result<(), StoreError> {
        require_real_directory(&self.root, "store root")?;
        verify_real_directory_within(&self.root, &self.staging_root, "staging root")?;
        verify_real_directory_within(&self.root, &self.source_staging_root, "source staging root")?;
        verify_real_directory_within(&self.root, &self.source_cache_root, "source cache root")?;
        verify_real_directory_within(&self.root, &self.lock_root, "lock root")?;
        verify_real_directory_within(&self.root, &self.results_sha256_root, "result root")?;
        Ok(())
    }

    fn lock_store(&self) -> Result<File, StoreError> {
        self.verify_layout()?;
        let file = open_lock_file(&self.store_lock_path)?;
        file.lock()
            .map_err(|source| io_error("lock store", &self.store_lock_path, source))?;
        Ok(file)
    }

    fn result_path(&self, id: &StoreResultId) -> PathBuf {
        self.results_sha256_root.join(id.hex())
    }
}

/// A leased, unverified workspace for source bytes. No publication method exists;
/// callers must independently verify and safely materialize locked sources.
pub struct UnverifiedSourceStage {
    store: Store,
    path: PathBuf,
    lock_path: PathBuf,
    lease: Option<File>,
    finished: bool,
}

impl UnverifiedSourceStage {
    /// The path is for untrusted staging only, never a build input or cache hit.
    #[must_use]
    pub fn untrusted_path(&self) -> &Path {
        &self.path
    }

    /// Stream an unverified archive into a private file with a strict byte cap.
    /// An interrupted or oversized transfer never creates a completed blob.
    /// This does not verify the SHA-256 declared by prep.lock.
    pub fn spool_unverified_archive(
        &self,
        input: &mut impl Read,
        maximum: u64,
    ) -> Result<u64, StoreError> {
        let _store_lock = self.store.lock_store()?;
        verify_real_directory_within(
            &self.store.source_staging_root,
            &self.path,
            "source staging transaction",
        )?;
        let completed = self.path.join("archive.blob");
        if node_exists(&completed)? {
            return Err(StoreError::InvalidPath(
                "source archive is already staged".to_owned(),
            ));
        }
        let incomplete = self.path.join("archive.part");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&incomplete)
            .map_err(|source| io_error("create private archive part", &incomplete, source))?;
        drop(_store_lock);

        let outcome = (|| {
            let mut total = 0u64;
            let mut buffer = [0u8; 8192];
            loop {
                let n = match input.read(&mut buffer) {
                    Ok(count) => count,
                    Err(source) if source.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(source) => {
                        return Err(io_error("read source archive", &incomplete, source));
                    }
                };
                if n == 0 {
                    break;
                }
                total = total
                    .checked_add(n as u64)
                    .ok_or(StoreError::SourceTooLarge)?;
                if total > maximum {
                    return Err(StoreError::SourceTooLarge);
                }
                file.write_all(&buffer[..n])
                    .map_err(|source| io_error("write source archive", &incomplete, source))?;
            }
            file.sync_all()
                .map_err(|source| io_error("sync source archive", &incomplete, source))?;
            verify_open_file_identity(&incomplete, &file)?;
            let _store_lock = self.store.lock_store()?;
            verify_real_directory_within(
                &self.store.source_staging_root,
                &self.path,
                "source staging transaction",
            )?;
            if node_exists(&completed)? {
                return Err(StoreError::InvalidPath(
                    "source archive is already staged".to_owned(),
                ));
            }
            fs::rename(&incomplete, &completed).map_err(|source| {
                io_error("complete unverified source archive", &completed, source)
            })?;
            Ok(total)
        })();
        if outcome.is_err() {
            let _ = fs::remove_file(&incomplete);
        }
        outcome
    }

    /// Open completed staged bytes as **unverified** input. The caller must
    /// independently match the full file against the immutable lock identity.
    pub fn open_unverified_archive(&self) -> Result<File, StoreError> {
        let _store_lock = self.store.lock_store()?;
        verify_real_directory_within(
            &self.store.source_staging_root,
            &self.path,
            "source staging transaction",
        )?;
        let path = self.path.join("archive.blob");
        let file = File::open(&path)
            .map_err(|source| io_error("open unverified source archive", &path, source))?;
        verify_open_file_identity(&path, &file)?;
        Ok(file)
    }

    #[cfg(test)]
    fn leave_abandoned(mut self) -> Result<(PathBuf, PathBuf), StoreError> {
        if let Some(lease) = self.lease.take() {
            lease.unlock().map_err(|source| {
                io_error("unlock source staging lease", &self.lock_path, source)
            })?;
        }
        self.finished = true;
        Ok((self.path.clone(), self.lock_path.clone()))
    }
}

impl Drop for UnverifiedSourceStage {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Ok(_store_lock) = self.store.lock_store() {
            if verify_real_directory_within(
                &self.store.source_staging_root,
                &self.path,
                "source staging transaction",
            )
            .is_ok()
            {
                let _ = fs::remove_dir_all(&self.path);
            }
            if let Some(lease) = self.lease.take() {
                let _ = release_and_remove_lock_file(lease, &self.lock_path);
            }
        }
    }
}

pub struct StoreTransaction {
    store: Store,
    staging_path: PathBuf,
    prefix: PathBuf,
    lock_path: PathBuf,
    lease: Option<File>,
    finished: bool,
}

impl StoreTransaction {
    #[must_use]
    pub fn prefix(&self) -> &Path {
        &self.prefix
    }

    #[must_use]
    pub fn staging_path(&self) -> &Path {
        &self.staging_path
    }

    pub fn commit(mut self, id: &StoreResultId) -> Result<PublishOutcome, StoreError> {
        let store_lock = self.store.lock_store()?;
        verify_real_directory_within(
            &self.store.staging_root,
            &self.staging_path,
            "staging transaction",
        )?;
        verify_real_directory_within(&self.staging_path, &self.prefix, "staging prefix")?;
        ensure_same_filesystem(&self.staging_path, &self.store.results_sha256_root)?;
        validate_output_tree(&self.prefix)?;
        sync_output_tree(&self.prefix)?;
        write_result_metadata(&self.staging_path, id)?;
        sync_directory(&self.staging_path)?;

        let destination = self.store.result_path(id);
        if node_exists(&destination)? {
            validate_existing_result(&destination, id, &self.store.results_sha256_root)?;
            fs::remove_dir_all(&self.staging_path).map_err(|source| {
                io_error(
                    "remove redundant staging result",
                    &self.staging_path,
                    source,
                )
            })?;
            self.finish_lease()?;
            drop(store_lock);
            self.finished = true;
            return Ok(PublishOutcome::Existing(PublishedResult {
                id: id.clone(),
                prefix: destination.join("prefix"),
                path: destination,
            }));
        }

        if let Err(source) = fs::rename(&self.staging_path, &destination) {
            if node_exists(&destination)? {
                validate_existing_result(&destination, id, &self.store.results_sha256_root)?;
                fs::remove_dir_all(&self.staging_path).map_err(|cleanup| {
                    io_error("remove raced staging result", &self.staging_path, cleanup)
                })?;
                self.finish_lease()?;
                drop(store_lock);
                self.finished = true;
                return Ok(PublishOutcome::Existing(PublishedResult {
                    id: id.clone(),
                    prefix: destination.join("prefix"),
                    path: destination,
                }));
            }
            return Err(io_error("atomically publish result", &destination, source));
        }

        if let Err(failure) =
            validate_existing_result(&destination, id, &self.store.results_sha256_root)
        {
            return self.fail_after_publish(&destination, failure, true);
        }
        if let Err(failure) = self.finish_lease() {
            return self.fail_after_publish(&destination, failure, false);
        }
        if let Err(failure) = sync_directory(&self.store.results_sha256_root) {
            return self.fail_after_publish(&destination, failure, false);
        }

        drop(store_lock);
        self.finished = true;

        Ok(PublishOutcome::Published(PublishedResult {
            id: id.clone(),
            prefix: destination.join("prefix"),
            path: destination,
        }))
    }

    fn fail_after_publish(
        &mut self,
        destination: &Path,
        failure: StoreError,
        cleanup_lease: bool,
    ) -> Result<PublishOutcome, StoreError> {
        if let Err(rollback) = self.rollback_published_result(destination) {
            self.finished = true;
            return Err(StoreError::CorruptResult {
                path: destination.to_path_buf(),
                message: format!("publication failed: {failure}; rollback failed: {rollback}"),
            });
        }

        if cleanup_lease && let Err(cleanup) = self.finish_lease() {
            self.finished = true;
            return Err(StoreError::CorruptResult {
                path: destination.to_path_buf(),
                message: format!(
                    "publication failed: {failure}; result rollback succeeded; lease cleanup failed: {cleanup}"
                ),
            });
        }

        self.finished = true;
        Err(failure)
    }

    fn rollback_published_result(&self, destination: &Path) -> Result<(), StoreError> {
        validate_result_directory(
            &self.store.results_sha256_root,
            destination,
            "published result rollback target",
        )?;
        fs::remove_dir_all(destination)
            .map_err(|source| io_error("roll back published result", destination, source))?;
        sync_directory(&self.store.results_sha256_root)
    }

    fn finish_lease(&mut self) -> Result<(), StoreError> {
        if let Some(lease) = self.lease.take() {
            release_and_remove_lock_file(lease, &self.lock_path)?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn leave_abandoned(mut self) -> Result<(PathBuf, PathBuf), StoreError> {
        if let Some(lease) = self.lease.take() {
            lease
                .unlock()
                .map_err(|source| io_error("unlock staging lease", &self.lock_path, source))?;
        }
        self.finished = true;
        Ok((self.staging_path.clone(), self.lock_path.clone()))
    }
}

impl Drop for StoreTransaction {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Ok(_store_lock) = self.store.lock_store() {
            let _ = fs::remove_dir_all(&self.staging_path);
            if let Some(lease) = self.lease.take() {
                let _ = release_and_remove_lock_file(lease, &self.lock_path);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedResult {
    id: StoreResultId,
    path: PathBuf,
    prefix: PathBuf,
}

impl PublishedResult {
    #[must_use]
    pub fn id(&self) -> &StoreResultId {
        &self.id
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn prefix(&self) -> &Path {
        &self.prefix
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishOutcome {
    Published(PublishedResult),
    Existing(PublishedResult),
}

impl PublishOutcome {
    #[must_use]
    pub fn result(&self) -> &PublishedResult {
        match self {
            Self::Published(result) | Self::Existing(result) => result,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationCollision {
    pub relative_path: PathBuf,
    pub result_ids: Vec<StoreResultId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationPlan {
    pub path_entries: Vec<PathBuf>,
    pub cmake_prefixes: Vec<PathBuf>,
    pub pkg_config_paths: Vec<PathBuf>,
    pub collisions: Vec<ActivationCollision>,
}

impl ActivationPlan {
    pub fn from_results(results: &[PublishedResult]) -> Result<Self, StoreError> {
        let mut seen_ids = BTreeSet::new();
        for result in results {
            if !seen_ids.insert(result.id.clone()) {
                return Err(StoreError::Activation(format!(
                    "duplicate result {} in activation plan",
                    result.id
                )));
            }
            validate_output_tree(&result.prefix)?;
        }

        let mut path_entries = Vec::new();
        let mut cmake_prefixes = Vec::new();
        let mut pkg_config_paths = Vec::new();
        let mut leaves: BTreeMap<PathBuf, Vec<StoreResultId>> = BTreeMap::new();

        for result in results {
            let prefix = &result.prefix;
            let bin = prefix.join("bin");
            if bin.is_dir() {
                path_entries.push(bin);
            }
            cmake_prefixes.push(prefix.clone());

            for candidate in [prefix.join("lib/pkgconfig"), prefix.join("share/pkgconfig")] {
                if candidate.is_dir() {
                    pkg_config_paths.push(candidate);
                }
            }

            collect_leaf_paths(prefix, prefix, &result.id, &mut leaves)?;
        }

        let collisions = leaves
            .into_iter()
            .filter_map(|(relative_path, result_ids)| {
                if result_ids.len() > 1 {
                    Some(ActivationCollision {
                        relative_path,
                        result_ids,
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(Self {
            path_entries,
            cmake_prefixes,
            pkg_config_paths,
            collisions,
        })
    }

    pub fn environment_overrides(
        &self,
        base: &BTreeMap<String, OsString>,
    ) -> Result<BTreeMap<String, OsString>, StoreError> {
        let mut output = BTreeMap::new();
        output.insert(
            "PATH".to_owned(),
            compose_path_value(&self.path_entries, base.get("PATH"))?,
        );
        output.insert(
            "CMAKE_PREFIX_PATH".to_owned(),
            compose_path_value(&self.cmake_prefixes, base.get("CMAKE_PREFIX_PATH"))?,
        );
        output.insert(
            "PKG_CONFIG_PATH".to_owned(),
            compose_path_value(&self.pkg_config_paths, base.get("PKG_CONFIG_PATH"))?,
        );
        Ok(output)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultMetadata {
    schema: String,
    result_id: String,
    state: String,
    retention: String,
}

fn write_result_metadata(staging_path: &Path, id: &StoreResultId) -> Result<(), StoreError> {
    let metadata = ResultMetadata {
        schema: RESULT_SCHEMA_V1.to_owned(),
        result_id: id.as_string(),
        state: "complete".to_owned(),
        retention: RESULT_RETENTION_PINNED.to_owned(),
    };
    let encoded = toml::to_string(&metadata).map_err(|error| {
        StoreError::Serialization(format!("serialize result metadata: {error}"))
    })?;
    let path = staging_path.join("result.toml");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|source| io_error("create result metadata", &path, source))?;
    file.write_all(encoded.as_bytes())
        .map_err(|source| io_error("write result metadata", &path, source))?;
    file.sync_all()
        .map_err(|source| io_error("sync result metadata", &path, source))
}

fn validate_existing_result(
    path: &Path,
    id: &StoreResultId,
    result_root: &Path,
) -> Result<(), StoreError> {
    validate_result_directory(result_root, path, "result directory")?;

    let metadata_path = path.join("result.toml");
    let metadata_node = fs::symlink_metadata(&metadata_path)
        .map_err(|source| io_error("inspect result metadata", &metadata_path, source))?;
    if !metadata_node.file_type().is_file() || metadata_node.file_type().is_symlink() {
        return Err(StoreError::CorruptResult {
            path: path.to_path_buf(),
            message: "result metadata must be a regular file".to_owned(),
        });
    }

    let encoded = fs::read_to_string(&metadata_path)
        .map_err(|source| io_error("read result metadata", &metadata_path, source))?;
    let metadata: ResultMetadata =
        toml::from_str(&encoded).map_err(|error| StoreError::CorruptResult {
            path: path.to_path_buf(),
            message: format!("invalid result metadata: {error}"),
        })?;
    if metadata.schema != RESULT_SCHEMA_V1
        || metadata.result_id != id.as_string()
        || metadata.state != "complete"
        || metadata.retention != RESULT_RETENTION_PINNED
    {
        return Err(StoreError::CorruptResult {
            path: path.to_path_buf(),
            message: "metadata does not identify a complete pinned expected result".to_owned(),
        });
    }

    let prefix = path.join("prefix");
    validate_result_directory(path, &prefix, "result prefix")?;
    validate_output_tree(&prefix)
}

fn validate_result_directory(parent: &Path, path: &Path, label: &str) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| StoreError::CorruptResult {
        path: path.to_path_buf(),
        message: format!("cannot inspect {label}: {source}"),
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(StoreError::CorruptResult {
            path: path.to_path_buf(),
            message: format!("{label} must be a real directory"),
        });
    }
    let canonical = fs::canonicalize(path).map_err(|source| StoreError::CorruptResult {
        path: path.to_path_buf(),
        message: format!("cannot canonicalize {label}: {source}"),
    })?;
    if canonical != path || canonical.strip_prefix(parent).is_err() {
        return Err(StoreError::CorruptResult {
            path: path.to_path_buf(),
            message: format!("{label} escapes its expected parent"),
        });
    }
    Ok(())
}

fn validate_output_tree(root: &Path) -> Result<(), StoreError> {
    require_output_root(root)?;
    visit_output_tree(root, root)
}

fn require_output_root(root: &Path) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|source| io_error("inspect output root", root, source))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(StoreError::InvalidOutput {
            path: root.to_path_buf(),
            message: "output root must be a real directory".to_owned(),
        });
    }
    Ok(())
}

fn visit_output_tree(root: &Path, directory: &Path) -> Result<(), StoreError> {
    for entry in fs::read_dir(directory)
        .map_err(|source| io_error("read output directory", directory, source))?
    {
        let entry = entry.map_err(|source| io_error("read output entry", directory, source))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| io_error("read output entry type", &path, source))?;

        if file_type.is_dir() {
            visit_output_tree(root, &path)?;
        } else if file_type.is_symlink() {
            validate_symlink(root, &path)?;
        } else if file_type.is_file() {
            #[cfg(unix)]
            {
                let metadata = entry
                    .metadata()
                    .map_err(|source| io_error("read output file metadata", &path, source))?;
                if metadata.nlink() > 1 {
                    return Err(StoreError::InvalidOutput {
                        path,
                        message: "hard-linked regular files are not permitted in v1 results"
                            .to_owned(),
                    });
                }
            }
        } else {
            return Err(StoreError::InvalidOutput {
                path,
                message: "special files are not permitted in result prefixes".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_symlink(root: &Path, path: &Path) -> Result<(), StoreError> {
    let target =
        fs::read_link(path).map_err(|source| io_error("read output symlink", path, source))?;
    if target.is_absolute() {
        return Err(StoreError::InvalidOutput {
            path: path.to_path_buf(),
            message: "absolute symlink target escapes the result prefix".to_owned(),
        });
    }

    let parent = path.parent().ok_or_else(|| StoreError::InvalidOutput {
        path: path.to_path_buf(),
        message: "symlink has no parent directory".to_owned(),
    })?;
    let relative_parent = parent
        .strip_prefix(root)
        .map_err(|_| StoreError::InvalidOutput {
            path: path.to_path_buf(),
            message: "symlink is outside the result prefix".to_owned(),
        })?;

    let mut components: Vec<OsString> = relative_parent
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_os_string()),
            _ => None,
        })
        .collect();

    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => components.push(value.to_os_string()),
            Component::ParentDir => {
                if components.pop().is_none() {
                    return Err(StoreError::InvalidOutput {
                        path: path.to_path_buf(),
                        message: "relative symlink target escapes the result prefix".to_owned(),
                    });
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(StoreError::InvalidOutput {
                    path: path.to_path_buf(),
                    message: "symlink target contains an absolute path prefix".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn sync_output_tree(root: &Path) -> Result<(), StoreError> {
    for entry in fs::read_dir(root)
        .map_err(|source| io_error("read output directory for sync", root, source))?
    {
        let entry = entry.map_err(|source| io_error("read output entry for sync", root, source))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| io_error("read output entry type for sync", &path, source))?;
        if file_type.is_dir() {
            sync_output_tree(&path)?;
        } else if file_type.is_file() {
            let file = File::open(&path)
                .map_err(|source| io_error("open output file for sync", &path, source))?;
            file.sync_all()
                .map_err(|source| io_error("sync output file", &path, source))?;
        } else if !file_type.is_symlink() {
            return Err(StoreError::InvalidOutput {
                path,
                message: "special files are not permitted in result prefixes".to_owned(),
            });
        }
    }
    sync_directory(root)
}

fn collect_leaf_paths(
    root: &Path,
    directory: &Path,
    result_id: &StoreResultId,
    leaves: &mut BTreeMap<PathBuf, Vec<StoreResultId>>,
) -> Result<(), StoreError> {
    for entry in fs::read_dir(directory)
        .map_err(|source| io_error("read activation directory", directory, source))?
    {
        let entry = entry.map_err(|source| io_error("read activation entry", directory, source))?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|source| io_error("read activation entry type", &path, source))?
            .is_dir()
        {
            collect_leaf_paths(root, &path, result_id, leaves)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| StoreError::Activation("activation path escaped prefix".to_owned()))?
                .to_path_buf();
            leaves.entry(relative).or_default().push(result_id.clone());
        }
    }
    Ok(())
}

fn compose_path_value(
    entries: &[PathBuf],
    base: Option<&OsString>,
) -> Result<OsString, StoreError> {
    let mut paths = entries.to_vec();
    if let Some(base) = base {
        paths.extend(std::env::split_paths(base));
    }
    std::env::join_paths(paths).map_err(|error| {
        StoreError::Activation(format!("cannot compose activation path list: {error}"))
    })
}

fn sync_directory(path: &Path) -> Result<(), StoreError> {
    let file =
        File::open(path).map_err(|source| io_error("open directory for sync", path, source))?;
    file.sync_all()
        .map_err(|source| io_error("sync directory", path, source))
}

fn ensure_child_directory(parent: &Path, name: &str) -> Result<PathBuf, StoreError> {
    let path = parent.join(name);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(StoreError::InvalidPath(format!(
                    "{} must be a real directory, not a symlink or other file type",
                    path.display()
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match fs::create_dir(&path) {
            Ok(()) => {}
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(io_error("create store directory", &path, source));
            }
        },
        Err(source) => return Err(io_error("inspect store directory", &path, source)),
    }

    verify_real_directory_within(parent, &path, "store child directory")?;
    fs::canonicalize(&path)
        .map_err(|source| io_error("canonicalize store directory", &path, source))
}

fn require_real_directory(path: &Path, label: &str) -> Result<(), StoreError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| io_error("inspect directory", path, source))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(StoreError::InvalidPath(format!(
            "{label} must be a real directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn verify_real_directory_within(parent: &Path, path: &Path, label: &str) -> Result<(), StoreError> {
    require_real_directory(path, label)?;
    let canonical = fs::canonicalize(path)
        .map_err(|source| io_error("canonicalize directory", path, source))?;
    if canonical != path || canonical.strip_prefix(parent).is_err() {
        return Err(StoreError::InvalidPath(format!(
            "{label} escapes its expected parent: {}",
            path.display()
        )));
    }
    Ok(())
}

fn create_new_lock_file(path: &Path) -> Result<File, StoreError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| io_error("create lock file", path, source))?;
    verify_open_file_identity(path, &file)?;
    Ok(file)
}

fn open_lock_file(path: &Path) -> Result<File, StoreError> {
    for _ in 0..4 {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                validate_lock_metadata(path, &metadata)?;
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)
                    .map_err(|source| io_error("open lock file", path, source))?;
                verify_open_file_identity(path, &file)?;
                return Ok(file);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match create_new_lock_file(path) {
                    Ok(file) => return Ok(file),
                    Err(StoreError::Io { source, .. })
                        if source.kind() == std::io::ErrorKind::AlreadyExists =>
                    {
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(source) => return Err(io_error("inspect lock file", path, source)),
        }
    }
    Err(StoreError::InvalidPath(format!(
        "lock file changed repeatedly while opening: {}",
        path.display()
    )))
}

fn open_existing_lock_file(path: &Path) -> Result<File, StoreError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| io_error("inspect existing lock file", path, source))?;
    validate_lock_metadata(path, &metadata)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| io_error("open existing lock file", path, source))?;
    verify_open_file_identity(path, &file)?;
    Ok(file)
}

fn validate_lock_metadata(path: &Path, metadata: &fs::Metadata) -> Result<(), StoreError> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(StoreError::InvalidPath(format!(
            "lock path must be a regular file: {}",
            path.display()
        )));
    }
    #[cfg(unix)]
    if metadata.nlink() != 1 {
        return Err(StoreError::InvalidPath(format!(
            "lock file must not have hard links: {}",
            path.display()
        )));
    }
    Ok(())
}

fn verify_open_file_identity(path: &Path, file: &File) -> Result<(), StoreError> {
    let path_metadata = fs::symlink_metadata(path)
        .map_err(|source| io_error("reinspect open file path", path, source))?;
    validate_lock_metadata(path, &path_metadata)?;
    let file_metadata = file
        .metadata()
        .map_err(|source| io_error("inspect open file", path, source))?;

    #[cfg(unix)]
    if path_metadata.dev() != file_metadata.dev() || path_metadata.ino() != file_metadata.ino() {
        return Err(StoreError::InvalidPath(format!(
            "lock path changed while opening: {}",
            path.display()
        )));
    }
    Ok(())
}

fn release_and_remove_lock_file(file: File, path: &Path) -> Result<(), StoreError> {
    verify_open_file_identity(path, &file)?;

    #[cfg(unix)]
    {
        fs::remove_file(path).map_err(|source| io_error("remove lock file", path, source))?;
        file.unlock()
            .map_err(|source| io_error("unlock removed lock file", path, source))?;
    }

    #[cfg(not(unix))]
    {
        file.unlock()
            .map_err(|source| io_error("unlock lock file", path, source))?;
        fs::remove_file(path).map_err(|source| io_error("remove lock file", path, source))?;
    }

    Ok(())
}

fn node_exists(path: &Path) -> Result<bool, StoreError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error("inspect filesystem node", path, source)),
    }
}

fn require_absolute(label: &str, path: &Path) -> Result<(), StoreError> {
    if !path.is_absolute() {
        return Err(StoreError::InvalidPath(format!(
            "{label} must be an absolute path"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_same_filesystem(left: &Path, right: &Path) -> Result<(), StoreError> {
    let left_metadata = fs::metadata(left)
        .map_err(|source| io_error("inspect staging filesystem", left, source))?;
    let right_metadata = fs::metadata(right)
        .map_err(|source| io_error("inspect result filesystem", right, source))?;
    if left_metadata.dev() != right_metadata.dev() {
        return Err(StoreError::CrossFilesystem);
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_same_filesystem(_left: &Path, _right: &Path) -> Result<(), StoreError> {
    Err(StoreError::CrossFilesystem)
}

fn canonical_source_sha256(value: &str) -> Result<String, StoreError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StoreError::InvalidResultId(
            "source cache digest must contain exactly 64 hexadecimal characters".to_owned(),
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn read_bounded_source_bytes(file: &mut impl Read, maximum: u64) -> Result<Vec<u8>, StoreError> {
    if maximum > MAX_CACHE_ARCHIVE_BYTES {
        return Err(StoreError::SourceTooLarge);
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| StoreError::Io {
            operation: "read bounded source bytes",
            path: PathBuf::from("<source file>"),
            source,
        })?;
    if bytes.len() as u64 > maximum {
        return Err(StoreError::SourceTooLarge);
    }
    Ok(bytes)
}

fn transaction_id() -> String {
    let counter = TRANSACTION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("txn-{}-{nanos:x}-{counter:x}", std::process::id())
}

fn valid_transaction_id(value: &str) -> bool {
    value.starts_with("txn-")
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn io_error(operation: &'static str, path: &Path, source: std::io::Error) -> StoreError {
    StoreError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "prep-store-test-{label}-{}-{:x}",
                std::process::id(),
                TRANSACTION_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(fs::canonicalize(path).expect("canonicalize test directory"))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn result_id(byte: &str) -> StoreResultId {
        StoreResultId::parse(format!("sha256:{}", byte.repeat(64))).expect("valid result id")
    }

    #[test]
    fn unverified_source_read_failures_clean_up_and_transient_interrupts_retry() {
        struct OneFailure {
            emitted: bool,
            fail_with: std::io::ErrorKind,
        }
        impl Read for OneFailure {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if !self.emitted {
                    self.emitted = true;
                    buffer[0] = b'x';
                    return Ok(1);
                }
                Err(self.fail_with.into())
            }
        }
        let temp = TestDirectory::new("source-spool-read-errors");
        let store = Store::open(&temp.0.join("store")).expect("store");
        let stage = store.begin_unverified_source().expect("stage");
        let mut failing = OneFailure {
            emitted: false,
            fail_with: std::io::ErrorKind::BrokenPipe,
        };
        assert!(stage.spool_unverified_archive(&mut failing, 8).is_err());
        assert!(!stage.untrusted_path().join("archive.part").exists());
        assert!(stage.open_unverified_archive().is_err());

        struct InterruptedOnce {
            inner: std::io::Cursor<&'static [u8]>,
            interrupted: bool,
        }
        impl Read for InterruptedOnce {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                self.inner.read(buffer)
            }
        }
        let mut input = InterruptedOnce {
            inner: std::io::Cursor::new(b"abc"),
            interrupted: false,
        };
        assert_eq!(stage.spool_unverified_archive(&mut input, 3).unwrap(), 3);
    }

    #[test]
    fn unverified_archive_spool_is_bounded_and_not_a_build_result() {
        let temp = TestDirectory::new("source-spool");
        let store = Store::open(&temp.0.join("store")).expect("open store");
        let stage = store.begin_unverified_source().expect("source stage");
        let oversized = &mut std::io::Cursor::new(b"abcdef");
        assert!(matches!(
            stage.spool_unverified_archive(oversized, 5),
            Err(StoreError::SourceTooLarge)
        ));
        assert!(stage.open_unverified_archive().is_err());
        assert!(!stage.untrusted_path().join("archive.part").exists());

        assert_eq!(
            stage
                .spool_unverified_archive(&mut std::io::Cursor::new(b"abc"), 3)
                .expect("spool within limit"),
            3
        );
        let mut file = stage
            .open_unverified_archive()
            .expect("completed unverified archive");
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).expect("read unverified bytes");
        assert_eq!(bytes, b"abc");
        assert!(
            stage
                .spool_unverified_archive(&mut std::io::Cursor::new(b"new"), 3)
                .is_err()
        );
        assert!(
            store
                .get(&result_id("9"))
                .expect("read result store")
                .is_none()
        );
        let path = stage.untrusted_path().to_owned();
        drop(file);
        drop(stage);
        assert!(!path.exists());
    }

    #[test]
    fn source_blob_cache_is_content_addressed_and_offline_reverified() {
        let temp = TestDirectory::new("verified-source-cache");
        let store = Store::open(&temp.0.join("store")).expect("store");
        let stage = store.begin_unverified_source().expect("stage");
        stage
            .spool_unverified_archive(&mut std::io::Cursor::new(b"abc"), 3)
            .expect("spool");
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
        assert!(matches!(
            store.cache_source_blob(&stage, wrong, 3),
            Err(StoreError::CorruptSource { .. })
        ));
        assert!(store.read_cached_source_blob(good, 3).unwrap().is_none());
        store.cache_source_blob(&stage, good, 3).expect("cache");
        drop(stage);
        assert_eq!(
            store.read_cached_source_blob(good, 3).unwrap().unwrap(),
            b"abc"
        );
        assert!(matches!(
            store.read_cached_source_blob(good, 2),
            Err(StoreError::SourceTooLarge)
        ));
        assert!(store.get(&result_id("9")).unwrap().is_none());
        let path = store.root().join("sources/sha256").join(good);
        fs::write(&path, b"tampered").expect("simulate corruption");
        assert!(matches!(
            store.read_cached_source_blob(good, 10),
            Err(StoreError::CorruptSource { .. })
        ));
    }

    #[test]
    fn cache_rejects_invalid_source_digest_and_foreign_stage() {
        let temp1 = TestDirectory::new("source-cache-origin");
        let temp2 = TestDirectory::new("source-cache-other");
        let first = Store::open(&temp1.0.join("store")).unwrap();
        let second = Store::open(&temp2.0.join("store")).unwrap();
        let stage = first.begin_unverified_source().unwrap();
        stage
            .spool_unverified_archive(&mut std::io::Cursor::new(b"abc"), 3)
            .unwrap();
        let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(second.cache_source_blob(&stage, digest, 3).is_err());
        assert!(first.read_cached_source_blob("not-a-digest", 3).is_err());
    }

    #[test]
    fn source_workspace_cannot_publish_build_results() {
        let temp = TestDirectory::new("source-separation");
        let store = Store::open(&temp.0.join("store")).expect("open store");
        let result = result_id("9");
        let path = {
            let source = store.begin_unverified_source().expect("source stage");
            let path = source.untrusted_path().to_owned();
            assert!(path.starts_with(store.root().join(".source-staging")));
            assert!(!path.starts_with(store.root().join(".staging")));
            fs::write(path.join("unverified.tar"), b"unsafe").expect("write unverified");
            assert!(store.get(&result).expect("query results").is_none());
            assert_eq!(store.recover_abandoned().expect("recover while active"), 0);
            path
        };
        assert!(!path.exists(), "source staging must clean up on drop");
        assert!(store.get(&result).expect("query results").is_none());
    }

    #[test]
    fn crashed_source_workspace_is_recovered_without_touching_active_sources() {
        let temp = TestDirectory::new("source-recovery");
        let store = Store::open(&temp.0.join("store")).expect("open store");
        let active = store.begin_unverified_source().expect("active source");
        let stale = store.begin_unverified_source().expect("stale source");
        let (path, lock_path) = stale.leave_abandoned().expect("simulate crash");
        assert!(path.exists());
        assert!(lock_path.exists());
        assert_eq!(store.recover_abandoned().expect("recover source"), 1);
        assert!(!path.exists());
        assert!(!lock_path.exists());
        assert!(active.untrusted_path().exists());
        drop(active);
    }

    #[cfg(unix)]
    #[test]
    fn source_staging_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let temp = TestDirectory::new("source-permissions");
        let store = Store::open(&temp.0.join("store")).expect("open store");
        let stage = store.begin_unverified_source().expect("source stage");
        let mode = fs::metadata(stage.untrusted_path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }

    #[test]
    fn user_paths_follow_explicit_xdg_roots() {
        let paths = UserPaths::resolve(
            Path::new("/home/test"),
            Some(Path::new("/data")),
            Some(Path::new("/cache")),
        )
        .expect("paths should resolve");
        assert_eq!(paths.store_root, Path::new("/data/prep/store"));
        assert_eq!(paths.cache_root, Path::new("/cache/prep"));
        assert!(UserPaths::resolve(Path::new("relative"), None, None).is_err());
    }

    #[test]
    fn successful_publish_is_isolated_idempotent_and_pinned() {
        let temp = TestDirectory::new("publish");
        let store = Store::open(&temp.0.join("store")).expect("open store");
        let id = result_id("a");

        let transaction = store.begin().expect("begin transaction");
        fs::create_dir_all(transaction.prefix().join("lib/pkgconfig")).expect("create output");
        fs::write(transaction.prefix().join("lib/pkgconfig/demo.pc"), b"first")
            .expect("write output");
        let first = transaction.commit(&id).expect("publish result");
        assert!(matches!(first, PublishOutcome::Published(_)));
        assert_eq!(
            fs::read(first.result().prefix().join("lib/pkgconfig/demo.pc")).expect("read result"),
            b"first"
        );

        let metadata: ResultMetadata = toml::from_str(
            &fs::read_to_string(first.result().path().join("result.toml"))
                .expect("read result metadata"),
        )
        .expect("parse result metadata");
        assert_eq!(metadata.retention, RESULT_RETENTION_PINNED);

        let second_transaction = store.begin().expect("begin second transaction");
        fs::write(second_transaction.prefix().join("other"), b"ignored")
            .expect("write redundant output");
        let second = second_transaction
            .commit(&id)
            .expect("reuse existing result");
        assert!(matches!(second, PublishOutcome::Existing(_)));
        assert!(!second.result().prefix().join("other").exists());
    }

    #[test]
    fn abandoned_transaction_is_recovered_but_active_transaction_is_not() {
        let temp = TestDirectory::new("recovery");
        let store = Store::open(&temp.0.join("store")).expect("open store");

        let active = store.begin().expect("begin active transaction");
        assert_eq!(store.recover_abandoned().expect("recovery should run"), 0);

        let abandoned = store.begin().expect("begin abandoned transaction");
        let (staging_path, lock_path) = abandoned.leave_abandoned().expect("leave abandoned");
        assert!(staging_path.exists());
        assert!(lock_path.exists());
        assert_eq!(store.recover_abandoned().expect("recover abandoned"), 1);
        assert!(!staging_path.exists());
        assert!(!lock_path.exists());

        drop(active);
    }

    #[cfg(unix)]
    #[test]
    fn escaping_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TestDirectory::new("symlink");
        let store = Store::open(&temp.0.join("store")).expect("open store");
        let transaction = store.begin().expect("begin transaction");
        symlink("../../outside", transaction.prefix().join("escape"))
            .expect("create escaping symlink");
        assert!(matches!(
            transaction.commit(&result_id("b")),
            Err(StoreError::InvalidOutput { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_file_is_rejected() {
        let temp = TestDirectory::new("hardlink");
        let outside = temp.0.join("outside");
        fs::write(&outside, b"host data").expect("write outside file");
        let store = Store::open(&temp.0.join("store")).expect("open store");
        let transaction = store.begin().expect("begin transaction");
        fs::hard_link(&outside, transaction.prefix().join("linked")).expect("create hard link");
        assert!(matches!(
            transaction.commit(&result_id("c")),
            Err(StoreError::InvalidOutput { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn preexisting_result_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TestDirectory::new("result-symlink");
        let store = Store::open(&temp.0.join("store")).expect("open store");
        let id = result_id("f");
        let outside = temp.0.join("outside-result");
        fs::create_dir_all(outside.join("prefix")).expect("create outside result");
        let destination = store.result_path(&id);
        symlink(&outside, &destination).expect("create hostile result symlink");

        assert!(matches!(
            store.get(&id),
            Err(StoreError::CorruptResult { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn replaced_result_prefix_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TestDirectory::new("prefix-symlink");
        let store = Store::open(&temp.0.join("store")).expect("open store");
        let id = result_id("1");
        let transaction = store.begin().expect("begin transaction");
        fs::write(transaction.prefix().join("tool"), b"tool").expect("write tool");
        let published = transaction
            .commit(&id)
            .expect("publish result")
            .result()
            .clone();

        fs::remove_dir_all(published.prefix()).expect("remove real prefix");
        let outside = temp.0.join("outside-prefix");
        fs::create_dir(&outside).expect("create outside prefix");
        symlink(&outside, published.prefix()).expect("replace prefix with symlink");

        assert!(matches!(
            store.get(&id),
            Err(StoreError::CorruptResult { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn hostile_store_lock_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TestDirectory::new("store-lock-symlink");
        let store_root = temp.0.join("store");
        fs::create_dir(&store_root).expect("create store root");
        let outside = temp.0.join("outside-lock");
        fs::write(&outside, b"outside").expect("create outside lock target");
        symlink(&outside, store_root.join(".store.lock")).expect("create hostile lock symlink");

        assert!(matches!(
            Store::open(&store_root),
            Err(StoreError::InvalidPath(_))
        ));
    }

    #[test]
    fn activation_reports_collisions_without_overwriting_results() {
        let temp = TestDirectory::new("activation");
        let store = Store::open(&temp.0.join("store")).expect("open store");
        let mut published = Vec::new();

        for (byte, contents) in [("d", b"one".as_slice()), ("e", b"two".as_slice())] {
            let transaction = store.begin().expect("begin transaction");
            fs::create_dir(transaction.prefix().join("bin")).expect("create bin");
            fs::write(transaction.prefix().join("bin/tool"), contents).expect("write tool");
            published.push(
                transaction
                    .commit(&result_id(byte))
                    .expect("publish result")
                    .result()
                    .clone(),
            );
        }

        let plan = ActivationPlan::from_results(&published).expect("build activation plan");
        assert_eq!(plan.path_entries.len(), 2);
        assert_eq!(plan.collisions.len(), 1);
        assert_eq!(plan.collisions[0].relative_path, Path::new("bin/tool"));
        assert_eq!(
            fs::read(published[0].prefix().join("bin/tool")).expect("read first tool"),
            b"one"
        );
        assert_eq!(
            fs::read(published[1].prefix().join("bin/tool")).expect("read second tool"),
            b"two"
        );
    }
}
