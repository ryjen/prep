use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const RESULT_SCHEMA_V1: &str = "prep.result/1";
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

        let staging_root = ensure_child_directory(&root, ".staging")?;
        let lock_root = ensure_child_directory(&root, ".locks")?;
        let results_root = ensure_child_directory(&root, "results")?;
        let results_sha256_root = ensure_child_directory(&results_root, "sha256")?;
        let store_lock_path = root.join(".store.lock");
        let _ = open_lock_file(&store_lock_path)?;

        Ok(Self {
            root,
            staging_root,
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
            let lease = match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => return Err(io_error("create staging lease", &lock_path, source)),
            };
            lease
                .lock()
                .map_err(|source| io_error("lock staging lease", &lock_path, source))?;

            let staging_path = self.staging_root.join(&id);
            match fs::create_dir(&staging_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = lease.unlock();
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                Err(source) => {
                    let _ = lease.unlock();
                    let _ = fs::remove_file(&lock_path);
                    return Err(io_error("create staging directory", &staging_path, source));
                }
            }

            let prefix = staging_path.join("prefix");
            if let Err(source) = fs::create_dir(&prefix) {
                let _ = fs::remove_dir_all(&staging_path);
                let _ = lease.unlock();
                let _ = fs::remove_file(&lock_path);
                return Err(io_error("create staging prefix", &prefix, source));
            }

            return Ok(StoreTransaction {
                store: self.clone(),
                id,
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

    pub fn get(&self, id: &StoreResultId) -> Result<Option<PublishedResult>, StoreError> {
        let path = self.result_path(id);
        if !path.exists() {
            return Ok(None);
        }
        validate_existing_result(&path, id)?;
        Ok(Some(PublishedResult {
            id: id.clone(),
            prefix: path.join("prefix"),
            path,
        }))
    }

    pub fn recover_abandoned(&self) -> Result<usize, StoreError> {
        let _store_lock = self.lock_store()?;
        let mut recovered = 0;

        for entry in fs::read_dir(&self.staging_root)
            .map_err(|source| io_error("read staging directory", &self.staging_root, source))?
        {
            let entry = entry
                .map_err(|source| io_error("read staging entry", &self.staging_root, source))?;
            let file_type = entry
                .file_type()
                .map_err(|source| io_error("read staging entry type", &entry.path(), source))?;
            if !file_type.is_dir() {
                return Err(StoreError::InvalidOutput {
                    path: entry.path(),
                    message: "staging root contains a non-directory entry".to_owned(),
                });
            }

            let Some(id) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                return Err(StoreError::InvalidOutput {
                    path: entry.path(),
                    message: "staging transaction name is not valid UTF-8".to_owned(),
                });
            };
            if !valid_transaction_id(&id) {
                return Err(StoreError::InvalidOutput {
                    path: entry.path(),
                    message: "staging transaction has an invalid identifier".to_owned(),
                });
            }

            let lock_path = self.lock_root.join(format!("{id}.lock"));
            let lease = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock_path)
                .map_err(|source| io_error("open staging lease", &lock_path, source))?;

            match lease.try_lock() {
                Ok(()) => {
                    fs::remove_dir_all(entry.path()).map_err(|source| {
                        io_error("remove abandoned staging directory", &entry.path(), source)
                    })?;
                    lease
                        .unlock()
                        .map_err(|source| io_error("unlock staging lease", &lock_path, source))?;
                    fs::remove_file(&lock_path).map_err(|source| {
                        io_error("remove abandoned staging lease", &lock_path, source)
                    })?;
                    recovered += 1;
                }
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(source)) => {
                    return Err(io_error("try staging lease", &lock_path, source));
                }
            }
        }

        self.remove_orphaned_unlocked_leases()?;
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
                continue;
            };
            if self.staging_root.join(stem).exists() {
                continue;
            }
            let lease = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|source| io_error("open orphan staging lease", &path, source))?;
            match lease.try_lock() {
                Ok(()) => {
                    lease
                        .unlock()
                        .map_err(|source| io_error("unlock orphan staging lease", &path, source))?;
                    fs::remove_file(&path)
                        .map_err(|source| io_error("remove orphan staging lease", &path, source))?;
                }
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(source)) => {
                    return Err(io_error("try orphan staging lease", &path, source));
                }
            }
        }
        Ok(())
    }

    fn lock_store(&self) -> Result<File, StoreError> {
        let file = open_lock_file(&self.store_lock_path)?;
        file.lock()
            .map_err(|source| io_error("lock store", &self.store_lock_path, source))?;
        Ok(file)
    }

    fn result_path(&self, id: &StoreResultId) -> PathBuf {
        self.results_sha256_root.join(id.hex())
    }
}

pub struct StoreTransaction {
    store: Store,
    id: String,
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
        ensure_same_filesystem(&self.staging_path, &self.store.results_sha256_root)?;
        validate_output_tree(&self.prefix)?;
        write_result_metadata(&self.staging_path, id)?;
        sync_tree_metadata(&self.staging_path)?;

        let destination = self.store.result_path(id);
        if destination.exists() {
            validate_existing_result(&destination, id)?;
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

        match fs::rename(&self.staging_path, &destination) {
            Ok(()) => {}
            Err(source) if destination.exists() => {
                validate_existing_result(&destination, id)?;
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
            Err(source) => return Err(io_error("atomically publish result", &destination, source)),
        }

        sync_directory(&self.store.results_sha256_root)?;
        self.finish_lease()?;
        drop(store_lock);
        self.finished = true;

        Ok(PublishOutcome::Published(PublishedResult {
            id: id.clone(),
            prefix: destination.join("prefix"),
            path: destination,
        }))
    }

    fn finish_lease(&mut self) -> Result<(), StoreError> {
        if let Some(lease) = self.lease.take() {
            lease
                .unlock()
                .map_err(|source| io_error("unlock staging lease", &self.lock_path, source))?;
        }
        if self.lock_path.exists() {
            fs::remove_file(&self.lock_path)
                .map_err(|source| io_error("remove staging lease", &self.lock_path, source))?;
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
                let _ = lease.unlock();
            }
            let _ = fs::remove_file(&self.lock_path);
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
}

fn write_result_metadata(staging_path: &Path, id: &StoreResultId) -> Result<(), StoreError> {
    let metadata = ResultMetadata {
        schema: RESULT_SCHEMA_V1.to_owned(),
        result_id: id.as_string(),
        state: "complete".to_owned(),
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

fn validate_existing_result(path: &Path, id: &StoreResultId) -> Result<(), StoreError> {
    let metadata_path = path.join("result.toml");
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
    {
        return Err(StoreError::CorruptResult {
            path: path.to_path_buf(),
            message: "metadata does not identify a complete expected result".to_owned(),
        });
    }
    let prefix = path.join("prefix");
    if !prefix.is_dir() {
        return Err(StoreError::CorruptResult {
            path: path.to_path_buf(),
            message: "result prefix is missing".to_owned(),
        });
    }
    validate_output_tree(&prefix)
}

fn validate_output_tree(root: &Path) -> Result<(), StoreError> {
    visit_output_tree(root, root)
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

fn sync_tree_metadata(staging_path: &Path) -> Result<(), StoreError> {
    sync_directory(&staging_path.join("prefix"))?;
    sync_directory(staging_path)
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
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&path)
                .map_err(|source| io_error("create store directory", &path, source))?;
        }
        Err(source) => return Err(io_error("inspect store directory", &path, source)),
    }

    let canonical = fs::canonicalize(&path)
        .map_err(|source| io_error("canonicalize store directory", &path, source))?;
    if canonical.strip_prefix(parent).is_err() {
        return Err(StoreError::InvalidPath(format!(
            "{} escapes its parent store root",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn open_lock_file(path: &Path) -> Result<File, StoreError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|source| io_error("open lock file", path, source))
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
            let _ = make_tree_writable(&self.0);
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn result_id(byte: &str) -> StoreResultId {
        StoreResultId::parse(format!("sha256:{}", byte.repeat(64))).expect("valid result id")
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
    fn successful_publish_is_isolated_and_idempotent() {
        let temp = TestDirectory::new("publish");
        let store = Store::open(&temp.0.join("store")).expect("open store");
        let id = result_id("a");

        let transaction = store.begin().expect("begin transaction");
        fs::create_dir(transaction.prefix().join("bin")).expect("create bin");
        fs::write(transaction.prefix().join("bin/tool"), b"first").expect("write output");
        let first = transaction.commit(&id).expect("publish result");
        assert!(matches!(first, PublishOutcome::Published(_)));
        assert_eq!(
            fs::read(first.result().prefix().join("bin/tool")).expect("read result"),
            b"first"
        );

        let second_transaction = store.begin().expect("begin second transaction");
        fs::write(second_transaction.prefix().join("other"), b"ignored")
            .expect("write redundant output");
        let second = second_transaction
            .commit(&id)
            .expect("reuse existing result");
        assert!(matches!(second, PublishOutcome::Existing(_)));
        assert_eq!(
            fs::read(second.result().prefix().join("bin/tool")).expect("read existing result"),
            b"first"
        );
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
            fs::read(published[0].prefix().join("bin/tool")).unwrap(),
            b"one"
        );
        assert_eq!(
            fs::read(published[1].prefix().join("bin/tool")).unwrap(),
            b"two"
        );
    }

    fn make_tree_writable(path: &Path) -> std::io::Result<()> {
        if !path.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                make_tree_writable(&entry.path())?;
            }
        }
        Ok(())
    }
}
