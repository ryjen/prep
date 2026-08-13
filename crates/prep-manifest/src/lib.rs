use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const MANIFEST_SCHEMA_V1: &str = "prep/1";
pub const LOCK_SCHEMA_V1: &str = "prep.lock/1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError {
    message: String,
}

impl ModelError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ModelError {}

macro_rules! validated_string_type {
    ($name:ident, $validator:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, ModelError> {
                let value = value.into();
                $validator(&value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

fn validate_name(value: &str) -> Result<(), ModelError> {
    if value.is_empty() {
        return Err(ModelError::new("identifier must not be empty"));
    }
    if value.len() > 128 {
        return Err(ModelError::new("identifier exceeds 128 bytes"));
    }
    if matches!(value, "." | "..") {
        return Err(ModelError::new("identifier must not be '.' or '..'"));
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
    }) {
        return Err(ModelError::new(
            "identifier may contain only ASCII letters, digits, '.', '_' and '-'",
        ));
    }
    if !value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(ModelError::new(
            "identifier must begin with an ASCII letter or digit",
        ));
    }
    Ok(())
}

fn validate_version(value: &str) -> Result<(), ModelError> {
    if value.is_empty() {
        return Err(ModelError::new("version must not be empty"));
    }
    if value.len() > 128 {
        return Err(ModelError::new("version exceeds 128 bytes"));
    }
    if matches!(value, "." | "..") {
        return Err(ModelError::new("version must not be '.' or '..'"));
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+')
    }) {
        return Err(ModelError::new(
            "version contains unsupported characters",
        ));
    }
    Ok(())
}

fn validate_reference(value: &str) -> Result<(), ModelError> {
    if value.is_empty() {
        return Err(ModelError::new("source reference must not be empty"));
    }
    if value.len() > 512 {
        return Err(ModelError::new("source reference exceeds 512 bytes"));
    }
    if value.chars().any(char::is_control) {
        return Err(ModelError::new("source reference contains control characters"));
    }
    Ok(())
}

fn validate_url(value: &str) -> Result<(), ModelError> {
    if value.is_empty() {
        return Err(ModelError::new("source URL must not be empty"));
    }
    if value.len() > 4096 {
        return Err(ModelError::new("source URL exceeds 4096 bytes"));
    }
    if value.chars().any(|character| character.is_control() || character.is_whitespace()) {
        return Err(ModelError::new(
            "source URL contains whitespace or control characters",
        ));
    }
    let has_scheme = value
        .split_once("://")
        .is_some_and(|(scheme, rest)| !scheme.is_empty() && !rest.is_empty());
    let is_ssh_scp_form = value.starts_with("git@") && value.contains(':');
    if !has_scheme && !is_ssh_scp_form {
        return Err(ModelError::new(
            "source URL must have a scheme or Git SSH scp form",
        ));
    }
    Ok(())
}

fn validate_local_path(value: &str) -> Result<(), ModelError> {
    if value.is_empty() {
        return Err(ModelError::new("local source path must not be empty"));
    }
    if value.len() > 4096 {
        return Err(ModelError::new("local source path exceeds 4096 bytes"));
    }
    if value.contains('\0') {
        return Err(ModelError::new("local source path contains NUL"));
    }
    Ok(())
}

validated_string_type!(PackageName, validate_name);
validated_string_type!(PluginName, validate_name);
validated_string_type!(PackageVersion, validate_version);
validated_string_type!(SourceReference, validate_reference);
validated_string_type!(SourceUrl, validate_url);
validated_string_type!(LocalSourcePath, validate_local_path);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct GitCommit(String);

impl GitCommit {
    pub fn parse(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into().to_ascii_lowercase();
        if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ModelError::new(
                "Git commit identity must be a 40- or 64-character hexadecimal object ID",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GitCommit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for GitCommit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn parse(value: impl Into<String>) -> Result<Self, ModelError> {
        let mut value = value.into().to_ascii_lowercase();
        if let Some(stripped) = value.strip_prefix("sha256:") {
            value = stripped.to_owned();
        }
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ModelError::new(
                "SHA-256 digest must contain exactly 64 hexadecimal characters",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn tagged(&self) -> String {
        format!("sha256:{}", self.0)
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.tagged())
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageMetadata {
    pub name: PackageName,
    pub version: PackageVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildDeclaration {
    #[serde(default)]
    pub systems: Vec<PluginName>,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceDeclaration {
    Git {
        url: SourceUrl,
        #[serde(rename = "ref", default)]
        reference: Option<SourceReference>,
    },
    Archive {
        url: SourceUrl,
        #[serde(default)]
        sha256: Option<Sha256Digest>,
    },
    Path {
        path: LocalSourcePath,
    },
}

impl SourceDeclaration {
    fn validate(&self) -> Result<(), ModelError> {
        if let Self::Archive { url, .. } = self
            && !url.as_str().starts_with("https://")
        {
            return Err(ModelError::new(
                "archive sources require an https:// URL in Prep 2 v1",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyDeclaration {
    pub name: PackageName,
    pub version: PackageVersion,
    pub source: SourceDeclaration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub package: PackageMetadata,
    #[serde(default)]
    pub build: BuildDeclaration,
    #[serde(default)]
    pub dependencies: Vec<DependencyDeclaration>,
}

impl Manifest {
    pub fn parse(input: &str) -> Result<Self, ModelError> {
        let manifest: Self = toml::from_str(input)
            .map_err(|error| ModelError::new(format!("invalid prep.toml: {error}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        if self.schema != MANIFEST_SCHEMA_V1 {
            return Err(ModelError::new(format!(
                "unsupported manifest schema {:?}; expected {MANIFEST_SCHEMA_V1}",
                self.schema
            )));
        }
        let mut names = BTreeSet::new();
        for dependency in &self.dependencies {
            dependency.source.validate()?;
            if !names.insert(dependency.name.clone()) {
                return Err(ModelError::new(format!(
                    "duplicate dependency {}",
                    dependency.name
                )));
            }
        }
        Ok(())
    }

    pub fn to_toml(&self) -> Result<String, ModelError> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized
            .dependencies
            .sort_by(|left, right| left.name.cmp(&right.name));
        normalized.build.systems.sort();
        toml::to_string_pretty(&normalized)
            .map_err(|error| ModelError::new(format!("failed to serialize prep.toml: {error}")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderIdentity {
    pub name: PluginName,
    pub version: PackageVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LockedSource {
    Git {
        url: SourceUrl,
        commit: GitCommit,
        #[serde(default)]
        requested_ref: Option<SourceReference>,
        provider: ProviderIdentity,
    },
    Archive {
        url: SourceUrl,
        sha256: Sha256Digest,
        provider: ProviderIdentity,
    },
    Path {
        path: LocalSourcePath,
    },
}

impl LockedSource {
    #[must_use]
    pub fn is_immutable(&self) -> bool {
        !matches!(self, Self::Path { .. })
    }

    #[must_use]
    pub fn is_globally_cacheable(&self) -> bool {
        self.is_immutable()
    }

    #[must_use]
    pub fn canonical_identity(&self) -> String {
        match self {
            Self::Git { url, commit, .. } => format!("git:{}@{}", url.as_str(), commit.as_str()),
            Self::Archive { url, sha256, .. } => {
                format!("archive:{}#{}", url.as_str(), sha256.tagged())
            }
            Self::Path { path } => format!("local:{}", path.as_str()),
        }
    }

    fn validate(&self) -> Result<(), ModelError> {
        if let Self::Archive { url, .. } = self
            && !url.as_str().starts_with("https://")
        {
            return Err(ModelError::new(
                "locked archive sources require an https:// URL in Prep 2 v1",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedRoot {
    pub name: PackageName,
    pub version: PackageVersion,
    #[serde(default)]
    pub dependencies: Vec<PackageName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedPackage {
    pub name: PackageName,
    pub version: PackageVersion,
    #[serde(default)]
    pub dependencies: Vec<PackageName>,
    pub source: LockedSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lockfile {
    pub schema: String,
    pub root: LockedRoot,
    #[serde(default, rename = "package")]
    pub packages: Vec<LockedPackage>,
}

impl Lockfile {
    pub fn parse(input: &str) -> Result<Self, ModelError> {
        let lockfile: Self = toml::from_str(input)
            .map_err(|error| ModelError::new(format!("invalid prep.lock: {error}")))?;
        lockfile.validate()?;
        Ok(lockfile)
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        if self.schema != LOCK_SCHEMA_V1 {
            return Err(ModelError::new(format!(
                "unsupported lock schema {:?}; expected {LOCK_SCHEMA_V1}",
                self.schema
            )));
        }
        let mut package_names = BTreeSet::new();
        for package in &self.packages {
            package.source.validate()?;
            if !package_names.insert(package.name.clone()) {
                return Err(ModelError::new(format!(
                    "duplicate locked package {}",
                    package.name
                )));
            }
            let mut dependencies = BTreeSet::new();
            for dependency in &package.dependencies {
                if !dependencies.insert(dependency.clone()) {
                    return Err(ModelError::new(format!(
                        "package {} has duplicate dependency {}",
                        package.name, dependency
                    )));
                }
            }
        }
        let mut root_dependencies = BTreeSet::new();
        for dependency in &self.root.dependencies {
            if !root_dependencies.insert(dependency.clone()) {
                return Err(ModelError::new(format!(
                    "root has duplicate dependency {dependency}"
                )));
            }
        }
        Ok(())
    }

    pub fn validate_against(&self, manifest: &Manifest) -> Result<(), ModelError> {
        self.validate()?;
        manifest.validate()?;
        if self.root.name != manifest.package.name || self.root.version != manifest.package.version {
            return Err(ModelError::new(
                "lock root package identity does not match manifest",
            ));
        }

        let declared: BTreeMap<_, _> = manifest
            .dependencies
            .iter()
            .map(|dependency| (dependency.name.clone(), dependency))
            .collect();
        let locked: BTreeMap<_, _> = self
            .packages
            .iter()
            .map(|package| (package.name.clone(), package))
            .collect();
        let root_dependencies: BTreeSet<_> = self.root.dependencies.iter().cloned().collect();
        let declared_names: BTreeSet<_> = declared.keys().cloned().collect();

        if root_dependencies != declared_names {
            return Err(ModelError::new(
                "lock root dependency set does not match manifest dependencies",
            ));
        }

        for (name, declaration) in declared {
            let package = locked.get(&name).ok_or_else(|| {
                ModelError::new(format!("manifest dependency {name} is missing from lockfile"))
            })?;
            if package.version != declaration.version {
                return Err(ModelError::new(format!(
                    "locked version for {name} does not match manifest"
                )));
            }
            validate_source_match(&declaration.source, &package.source).map_err(|error| {
                ModelError::new(format!("locked source for {name} does not match manifest: {error}"))
            })?;
        }
        Ok(())
    }

    pub fn to_toml(&self) -> Result<String, ModelError> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized.root.dependencies.sort();
        normalized.packages.sort_by(|left, right| left.name.cmp(&right.name));
        for package in &mut normalized.packages {
            package.dependencies.sort();
        }
        toml::to_string_pretty(&normalized)
            .map_err(|error| ModelError::new(format!("failed to serialize prep.lock: {error}")))
    }
}

fn validate_source_match(
    declaration: &SourceDeclaration,
    locked: &LockedSource,
) -> Result<(), ModelError> {
    match (declaration, locked) {
        (
            SourceDeclaration::Git { url, reference },
            LockedSource::Git {
                url: locked_url,
                requested_ref,
                ..
            },
        ) if url == locked_url && reference == requested_ref => Ok(()),
        (
            SourceDeclaration::Archive { url, sha256 },
            LockedSource::Archive {
                url: locked_url,
                sha256: locked_digest,
                ..
            },
        ) if url == locked_url && sha256.as_ref().is_none_or(|digest| digest == locked_digest) => {
            Ok(())
        }
        (
            SourceDeclaration::Path { path },
            LockedSource::Path { path: locked_path },
        ) if path == locked_path => Ok(()),
        _ => Err(ModelError::new("source kind or declared identity differs")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
schema = "prep/1"

[package]
name = "hello"
version = "1.0.0"

[build]
systems = ["cmake", "ninja"]

[[dependencies]]
name = "fmt"
version = "11.2.0"

[dependencies.source]
kind = "git"
url = "https://github.com/fmtlib/fmt"
ref = "11.2.0"
"#;

    const LOCK: &str = r#"
schema = "prep.lock/1"

[root]
name = "hello"
version = "1.0.0"
dependencies = ["fmt"]

[[package]]
name = "fmt"
version = "11.2.0"
dependencies = []

[package.source]
kind = "git"
url = "https://github.com/fmtlib/fmt"
commit = "0123456789abcdef0123456789abcdef01234567"
requested_ref = "11.2.0"

[package.source.provider]
name = "builtin.git"
version = "0.1.0"
"#;

    #[test]
    fn identifiers_reject_path_like_and_control_values() {
        for invalid in ["", ".", "..", "a/b", "a\\b", " bad", "bad\nname"] {
            assert!(PackageName::parse(invalid).is_err(), "accepted {invalid:?}");
        }
        for valid in ["fmt", "OpenSSL", "lib.foo", "foo_bar", "foo-bar"] {
            assert!(PackageName::parse(valid).is_ok(), "rejected {valid:?}");
        }
    }

    #[test]
    fn manifest_and_lock_round_trip_deterministically() {
        let manifest = Manifest::parse(MANIFEST).expect("manifest should parse");
        let first = manifest.to_toml().expect("manifest should serialize");
        let second = Manifest::parse(&first)
            .expect("serialized manifest should parse")
            .to_toml()
            .expect("manifest should reserialize");
        assert_eq!(first, second);

        let lock = Lockfile::parse(LOCK).expect("lock should parse");
        lock.validate_against(&manifest)
            .expect("lock should match manifest");
        let first = lock.to_toml().expect("lock should serialize");
        let second = Lockfile::parse(&first)
            .expect("serialized lock should parse")
            .to_toml()
            .expect("lock should reserialize");
        assert_eq!(first, second);
    }

    #[test]
    fn unknown_schema_and_unknown_fields_fail_closed() {
        assert!(Manifest::parse(&MANIFEST.replace("prep/1", "prep/99")).is_err());
        assert!(Manifest::parse(&MANIFEST.replace("version = \"1.0.0\"", "version = \"1.0.0\"\nextra = true")).is_err());
    }

    #[test]
    fn archive_requires_https() {
        let input = MANIFEST.replace(
            "kind = \"git\"\nurl = \"https://github.com/fmtlib/fmt\"\nref = \"11.2.0\"",
            "kind = \"archive\"\nurl = \"http://example.invalid/fmt.tar.gz\"\nsha256 = \"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"",
        );
        assert!(Manifest::parse(&input).is_err());
    }

    #[test]
    fn local_source_is_never_globally_cacheable() {
        let source = LockedSource::Path {
            path: LocalSourcePath::parse("../fmt").expect("local path should parse"),
        };
        assert!(!source.is_immutable());
        assert!(!source.is_globally_cacheable());
    }

    #[test]
    fn lock_source_mismatch_is_rejected() {
        let manifest = Manifest::parse(MANIFEST).expect("manifest should parse");
        let lock = Lockfile::parse(&LOCK.replace("requested_ref = \"11.2.0\"", "requested_ref = \"main\""))
            .expect("lock remains structurally valid");
        assert!(lock.validate_against(&manifest).is_err());
    }
}
