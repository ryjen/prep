use crate::plugin::PluginAdmissionPolicy;
use prep_manifest::{PackageName, PackageVersion};
use prep_protocol::{
    BuilderOperation, BuilderRequestFrame, PluginCapability, PluginExecutionContext, PluginPackage,
};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum PluginInputError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidDirectory {
        label: &'static str,
        path: PathBuf,
        message: String,
    },
    NonUtf8Path {
        label: &'static str,
        path: PathBuf,
    },
    InvalidEnvironmentKey(String),
    InvalidEnvironmentValue(String),
    MissingDeclaredCapability {
        operation: BuilderOperation,
        capability: PluginCapability,
    },
    PolicyDenied {
        operation: BuilderOperation,
        capability: PluginCapability,
    },
}

impl fmt::Display for PluginInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {}: {source}", path.display()),
            Self::InvalidDirectory {
                label,
                path,
                message,
            } => write!(formatter, "{label} {} is invalid: {message}", path.display()),
            Self::NonUtf8Path { label, path } => {
                write!(formatter, "{label} is not valid UTF-8: {}", path.display())
            }
            Self::InvalidEnvironmentKey(key) => {
                write!(formatter, "invalid plugin environment key {key:?}")
            }
            Self::InvalidEnvironmentValue(key) => {
                write!(formatter, "plugin environment value for {key:?} contains NUL")
            }
            Self::MissingDeclaredCapability {
                operation,
                capability,
            } => write!(
                formatter,
                "builder operation {operation} requires undeclared capability {capability}"
            ),
            Self::PolicyDenied {
                operation,
                capability,
            } => write!(
                formatter,
                "builder operation {operation} requires policy-denied capability {capability}"
            ),
        }
    }
}

impl Error for PluginInputError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginExecutionRoots {
    source_dir: PathBuf,
    build_dir: PathBuf,
    staging_dir: PathBuf,
}

impl PluginExecutionRoots {
    pub fn new(
        source_dir: &Path,
        build_dir: &Path,
        staging_dir: &Path,
    ) -> Result<Self, PluginInputError> {
        Ok(Self {
            source_dir: canonical_real_directory("source directory", source_dir)?,
            build_dir: canonical_real_directory("build directory", build_dir)?,
            staging_dir: canonical_real_directory("staging directory", staging_dir)?,
        })
    }

    #[must_use]
    pub fn source_dir(&self) -> &Path {
        &self.source_dir
    }

    #[must_use]
    pub fn build_dir(&self) -> &Path {
        &self.build_dir
    }

    #[must_use]
    pub fn staging_dir(&self) -> &Path {
        &self.staging_dir
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginEnvironmentPolicy {
    allowed_keys: BTreeSet<String>,
}

impl PluginEnvironmentPolicy {
    #[must_use]
    pub fn deny_all() -> Self {
        Self::default()
    }

    pub fn allow<I, S>(keys: I) -> Result<Self, PluginInputError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut allowed_keys = BTreeSet::new();
        for key in keys {
            let key = key.into();
            validate_environment_key(&key)?;
            allowed_keys.insert(key);
        }
        Ok(Self { allowed_keys })
    }

    pub fn sanitize(
        &self,
        candidates: &BTreeMap<String, String>,
    ) -> Result<SanitizedPluginEnvironment, PluginInputError> {
        let mut values = BTreeMap::new();
        for key in &self.allowed_keys {
            let Some(value) = candidates.get(key) else {
                continue;
            };
            if value.contains('\0') {
                return Err(PluginInputError::InvalidEnvironmentValue(key.clone()));
            }
            values.insert(key.clone(), value.clone());
        }
        Ok(SanitizedPluginEnvironment { values })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SanitizedPluginEnvironment {
    values: BTreeMap<String, String>,
}

impl SanitizedPluginEnvironment {
    #[must_use]
    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.values
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuilderRequestInput {
    pub id: String,
    pub operation: BuilderOperation,
    pub package_name: PackageName,
    pub package_version: PackageVersion,
    pub roots: PluginExecutionRoots,
    pub dependency_prefixes: Vec<PathBuf>,
    pub environment: SanitizedPluginEnvironment,
    pub offline: bool,
    pub arguments: Vec<String>,
}

impl BuilderRequestInput {
    pub fn into_frame(self) -> Result<BuilderRequestFrame, PluginInputError> {
        let dependency_prefixes = self
            .dependency_prefixes
            .iter()
            .map(|path| {
                canonical_real_directory("dependency prefix", path)
                    .and_then(|path| path_to_protocol_string("dependency prefix", &path))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(BuilderRequestFrame::new(
            self.id,
            self.operation,
            PluginPackage {
                name: self.package_name.as_str().to_owned(),
                version: self.package_version.as_str().to_owned(),
            },
            PluginExecutionContext {
                source_dir: path_to_protocol_string("source directory", self.roots.source_dir())?,
                build_dir: path_to_protocol_string("build directory", self.roots.build_dir())?,
                staging_dir: path_to_protocol_string("staging directory", self.roots.staging_dir())?,
                dependency_prefixes,
                environment: self.environment.values,
                offline: self.offline,
            },
            self.arguments,
        ))
    }
}

pub fn authorize_builder_operation(
    operation: BuilderOperation,
    declared_capabilities: &[PluginCapability],
    policy: &PluginAdmissionPolicy,
) -> Result<(), PluginInputError> {
    for capability in required_builder_capabilities(operation) {
        if !declared_capabilities.contains(capability) {
            return Err(PluginInputError::MissingDeclaredCapability {
                operation,
                capability: *capability,
            });
        }
        if !policy.allows(*capability) {
            return Err(PluginInputError::PolicyDenied {
                operation,
                capability: *capability,
            });
        }
    }
    Ok(())
}

#[must_use]
pub fn required_builder_capabilities(
    operation: BuilderOperation,
) -> &'static [PluginCapability] {
    const READ_AND_SPAWN: &[PluginCapability] = &[
        PluginCapability::ProcessSpawn,
        PluginCapability::FilesystemReadSource,
    ];
    const INSTALL: &[PluginCapability] = &[
        PluginCapability::ProcessSpawn,
        PluginCapability::FilesystemReadSource,
        PluginCapability::FilesystemWriteStaging,
    ];

    match operation {
        BuilderOperation::Configure | BuilderOperation::Build | BuilderOperation::Test => {
            READ_AND_SPAWN
        }
        BuilderOperation::InstallToStaging => INSTALL,
    }
}

fn canonical_real_directory(
    label: &'static str,
    path: &Path,
) -> Result<PathBuf, PluginInputError> {
    if !path.is_absolute() {
        return Err(PluginInputError::InvalidDirectory {
            label,
            path: path.to_path_buf(),
            message: "path must be absolute".to_owned(),
        });
    }

    let metadata = fs::symlink_metadata(path).map_err(|source| PluginInputError::Io {
        operation: "inspect directory",
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(PluginInputError::InvalidDirectory {
            label,
            path: path.to_path_buf(),
            message: "path must name a real directory, not a symlink or other node".to_owned(),
        });
    }

    fs::canonicalize(path).map_err(|source| PluginInputError::Io {
        operation: "canonicalize directory",
        path: path.to_path_buf(),
        source,
    })
}

fn path_to_protocol_string(
    label: &'static str,
    path: &Path,
) -> Result<String, PluginInputError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| PluginInputError::NonUtf8Path {
            label,
            path: path.to_path_buf(),
        })
}

fn validate_environment_key(key: &str) -> Result<(), PluginInputError> {
    if key.is_empty() || key.contains(['=', '\0']) {
        return Err(PluginInputError::InvalidEnvironmentKey(key.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "prep-plugin-input-{label}-{}-{}",
                std::process::id(),
                TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(fs::canonicalize(path).expect("canonicalize test directory"))
        }

        fn child(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir(&path).expect("create child directory");
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn execution_roots_require_real_absolute_directories() {
        assert!(matches!(
            PluginExecutionRoots::new(
                Path::new("relative-source"),
                Path::new("/build"),
                Path::new("/staging")
            ),
            Err(PluginInputError::InvalidDirectory { .. })
        ));
    }

    #[test]
    fn environment_policy_filters_ambient_candidates() {
        let policy =
            PluginEnvironmentPolicy::allow(["PATH", "CMAKE_PREFIX_PATH"]).expect("valid policy");
        let candidates = BTreeMap::from([
            ("PATH".to_owned(), "/tools/bin".to_owned()),
            ("LD_PRELOAD".to_owned(), "/tmp/inject.so".to_owned()),
            ("CMAKE_PREFIX_PATH".to_owned(), "/deps".to_owned()),
        ]);

        let environment = policy.sanitize(&candidates).expect("sanitize environment");

        assert_eq!(
            environment.as_map(),
            &BTreeMap::from([
                ("CMAKE_PREFIX_PATH".to_owned(), "/deps".to_owned()),
                ("PATH".to_owned(), "/tools/bin".to_owned()),
            ])
        );
        assert!(!environment.as_map().contains_key("LD_PRELOAD"));
    }

    #[test]
    fn environment_policy_rejects_invalid_selected_values() {
        let policy = PluginEnvironmentPolicy::allow(["PATH"]).expect("valid policy");
        let candidates = BTreeMap::from([("PATH".to_owned(), "bad\0value".to_owned())]);

        assert!(matches!(
            policy.sanitize(&candidates),
            Err(PluginInputError::InvalidEnvironmentValue(key)) if key == "PATH"
        ));
    }

    #[test]
    fn builder_request_uses_validated_canonical_inputs() {
        let temp = TestDirectory::new("request");
        let source = temp.child("source");
        let build = temp.child("build");
        let staging = temp.child("staging");
        let dependency = temp.child("dependency");

        let roots =
            PluginExecutionRoots::new(&source, &build, &staging).expect("validate execution roots");
        let environment = PluginEnvironmentPolicy::allow(["PATH"])
            .expect("valid policy")
            .sanitize(&BTreeMap::from([(
                "PATH".to_owned(),
                "/tools/bin".to_owned(),
            )]))
            .expect("sanitize environment");

        let frame = BuilderRequestInput {
            id: "42".to_owned(),
            operation: BuilderOperation::Configure,
            package_name: PackageName::parse("hello").expect("package name"),
            package_version: PackageVersion::parse("1.0.0").expect("package version"),
            roots,
            dependency_prefixes: vec![dependency.clone()],
            environment,
            offline: true,
            arguments: vec!["-DCMAKE_BUILD_TYPE=Release".to_owned()],
        }
        .into_frame()
        .expect("construct builder request");

        assert_eq!(frame.operation, BuilderOperation::Configure);
        assert_eq!(frame.package.name, "hello");
        assert_eq!(
            frame.context.dependency_prefixes,
            vec![dependency.to_string_lossy().into_owned()]
        );
        assert_eq!(frame.context.environment.get("PATH"), Some(&"/tools/bin".to_owned()));
        assert!(frame.context.offline);
    }

    #[test]
    fn operation_authority_requires_declared_and_allowed_capabilities() {
        let operation = BuilderOperation::Configure;
        let required = required_builder_capabilities(operation);

        let error = authorize_builder_operation(
            operation,
            &[PluginCapability::ProcessSpawn],
            &PluginAdmissionPolicy::allow(required.iter().copied()),
        )
        .expect_err("missing declaration must fail");
        assert!(matches!(
            error,
            PluginInputError::MissingDeclaredCapability {
                capability: PluginCapability::FilesystemReadSource,
                ..
            }
        ));

        let error = authorize_builder_operation(
            operation,
            required,
            &PluginAdmissionPolicy::allow([PluginCapability::ProcessSpawn]),
        )
        .expect_err("policy denial must fail");
        assert!(matches!(
            error,
            PluginInputError::PolicyDenied {
                capability: PluginCapability::FilesystemReadSource,
                ..
            }
        ));

        authorize_builder_operation(
            operation,
            required,
            &PluginAdmissionPolicy::allow(required.iter().copied()),
        )
        .expect("declared and allowed capabilities should pass");
    }
}
