use prep_manifest::{LockedSource, PackageName, PackageVersion, PluginName, Sha256Digest};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResultId(String);

impl ResultId {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        let Some(digest) = value.strip_prefix("sha256:") else {
            return Err(IdentityError::Invalid(
                "result identity must use sha256:<hex> form".to_owned(),
            ));
        };
        Sha256Digest::parse(digest)
            .map_err(|error| IdentityError::Invalid(error.to_string()))?;
        Ok(Self(format!("sha256:{}", digest.to_ascii_lowercase())))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ResultId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolIdentity {
    pub role: String,
    pub executable: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginBuildIdentity {
    pub name: PluginName,
    pub version: PackageVersion,
    pub content_digest: Sha256Digest,
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyResult {
    pub package: PackageName,
    pub result: ResultId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInput {
    pub source: LockedSource,
    pub dependencies: Vec<DependencyResult>,
    pub target: String,
    pub toolchain: ToolIdentity,
    pub build_tools: Vec<ToolIdentity>,
    pub plugin: PluginBuildIdentity,
    pub configuration: BTreeMap<String, String>,
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    NonCacheableSource,
    Invalid(String),
    DuplicateDependency(String),
    DuplicateToolRole(String),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonCacheableSource => write!(
                formatter,
                "local development source is not eligible for a reusable result identity"
            ),
            Self::Invalid(message) => formatter.write_str(message),
            Self::DuplicateDependency(package) => {
                write!(formatter, "duplicate dependency result for {package}")
            }
            Self::DuplicateToolRole(role) => {
                write!(formatter, "duplicate output-affecting tool role {role}")
            }
        }
    }
}

impl Error for IdentityError {}

impl BuildInput {
    pub fn result_identity(&self) -> Result<ResultId, IdentityError> {
        if !self.source.is_globally_cacheable() {
            return Err(IdentityError::NonCacheableSource);
        }
        validate_scalar("target", &self.target)?;
        validate_tool(&self.toolchain)?;
        validate_scalar("plugin protocol", &self.plugin.protocol)?;

        let mut dependencies = self.dependencies.clone();
        dependencies.sort_by(|left, right| left.package.cmp(&right.package));
        let mut dependency_names = BTreeSet::new();
        for dependency in &dependencies {
            if !dependency_names.insert(dependency.package.clone()) {
                return Err(IdentityError::DuplicateDependency(
                    dependency.package.to_string(),
                ));
            }
        }

        let mut tools = self.build_tools.clone();
        tools.sort_by(|left, right| left.role.cmp(&right.role));
        let mut tool_roles = BTreeSet::new();
        for tool in &tools {
            validate_tool(tool)?;
            if !tool_roles.insert(tool.role.clone()) {
                return Err(IdentityError::DuplicateToolRole(tool.role.clone()));
            }
        }

        for (key, value) in self.configuration.iter().chain(&self.environment) {
            validate_scalar("build input key", key)?;
            validate_scalar("build input value", value)?;
        }

        let mut hasher = IdentityHasher::new();
        hasher.field("schema", "prep.result/1");
        hasher.field("source", &self.source.canonical_identity());
        hasher.field("target", &self.target);
        hash_tool(&mut hasher, "toolchain", &self.toolchain);

        hasher.count("dependency.count", dependencies.len());
        for dependency in &dependencies {
            hasher.field("dependency.package", dependency.package.as_str());
            hasher.field("dependency.result", dependency.result.as_str());
        }

        hasher.count("build_tool.count", tools.len());
        for tool in &tools {
            hash_tool(&mut hasher, "build_tool", tool);
        }

        hasher.field("plugin.name", self.plugin.name.as_str());
        hasher.field("plugin.version", self.plugin.version.as_str());
        hasher.field("plugin.digest", &self.plugin.content_digest.tagged());
        hasher.field("plugin.protocol", &self.plugin.protocol);

        hash_map(&mut hasher, "configuration", &self.configuration);
        hash_map(&mut hasher, "environment", &self.environment);

        Ok(ResultId(format!("sha256:{}", hasher.finish_hex())))
    }
}

fn validate_tool(tool: &ToolIdentity) -> Result<(), IdentityError> {
    validate_scalar("tool role", &tool.role)?;
    validate_scalar("tool executable", &tool.executable)?;
    validate_scalar("tool version", &tool.version)
}

fn validate_scalar(label: &str, value: &str) -> Result<(), IdentityError> {
    if value.is_empty() {
        return Err(IdentityError::Invalid(format!("{label} must not be empty")));
    }
    if value.len() > 16 * 1024 {
        return Err(IdentityError::Invalid(format!("{label} is unreasonably large")));
    }
    if value.contains('\0') {
        return Err(IdentityError::Invalid(format!("{label} contains NUL")));
    }
    Ok(())
}

fn hash_tool(hasher: &mut IdentityHasher, prefix: &str, tool: &ToolIdentity) {
    hasher.field(&format!("{prefix}.role"), &tool.role);
    hasher.field(&format!("{prefix}.executable"), &tool.executable);
    hasher.field(&format!("{prefix}.version"), &tool.version);
}

fn hash_map(hasher: &mut IdentityHasher, prefix: &str, values: &BTreeMap<String, String>) {
    hasher.count(&format!("{prefix}.count"), values.len());
    for (key, value) in values {
        hasher.field(&format!("{prefix}.key"), key);
        hasher.field(&format!("{prefix}.value"), value);
    }
}

struct IdentityHasher(Sha256);

impl IdentityHasher {
    fn new() -> Self {
        Self(Sha256::new())
    }

    fn field(&mut self, label: &str, value: &str) {
        self.bytes(label.as_bytes());
        self.bytes(value.as_bytes());
    }

    fn count(&mut self, label: &str, value: usize) {
        self.field(label, &value.to_string());
    }

    fn bytes(&mut self, value: &[u8]) {
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value);
    }

    fn finish_hex(self) -> String {
        let digest = self.0.finalize();
        let mut output = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prep_manifest::{GitCommit, ProviderIdentity, SourceUrl};

    fn base() -> BuildInput {
        BuildInput {
            source: LockedSource::Git {
                url: SourceUrl::parse("https://example.invalid/lib.git").expect("url"),
                commit: GitCommit::parse("0123456789abcdef0123456789abcdef01234567")
                    .expect("commit"),
                requested_ref: None,
                provider: ProviderIdentity {
                    name: PluginName::parse("builtin.git").expect("provider name"),
                    version: PackageVersion::parse("0.1.0").expect("provider version"),
                },
            },
            dependencies: vec![DependencyResult {
                package: PackageName::parse("dep").expect("package"),
                result: ResultId::parse(format!("sha256:{}", "11".repeat(32)))
                    .expect("result id"),
            }],
            target: "x86_64-unknown-linux-gnu".to_owned(),
            toolchain: ToolIdentity {
                role: "compiler".to_owned(),
                executable: "/usr/bin/clang++".to_owned(),
                version: "clang 20.1".to_owned(),
            },
            build_tools: vec![ToolIdentity {
                role: "cmake".to_owned(),
                executable: "/usr/bin/cmake".to_owned(),
                version: "cmake 4.1".to_owned(),
            }],
            plugin: PluginBuildIdentity {
                name: PluginName::parse("cmake").expect("plugin name"),
                version: PackageVersion::parse("0.1.0").expect("plugin version"),
                content_digest: Sha256Digest::parse("22".repeat(32)).expect("plugin digest"),
                protocol: "prep.plugin/1".to_owned(),
            },
            configuration: BTreeMap::from([("build_type".to_owned(), "release".to_owned())]),
            environment: BTreeMap::from([("CXXFLAGS".to_owned(), "-O2".to_owned())]),
        }
    }

    #[test]
    fn identity_is_stable_for_equivalent_input_order() {
        let first = base();
        let mut second = base();
        second.dependencies.insert(
            0,
            DependencyResult {
                package: PackageName::parse("alpha").expect("package"),
                result: ResultId::parse(format!("sha256:{}", "33".repeat(32)))
                    .expect("result id"),
            },
        );
        let mut first_with_alpha = first.clone();
        first_with_alpha.dependencies.push(second.dependencies[0].clone());
        assert_eq!(
            first_with_alpha.result_identity().expect("identity"),
            second.result_identity().expect("identity")
        );
    }

    #[test]
    fn material_build_inputs_change_identity() {
        let baseline = base().result_identity().expect("baseline identity");

        let mut compiler = base();
        compiler.toolchain.version = "clang 21.0".to_owned();
        assert_ne!(baseline, compiler.result_identity().expect("compiler identity"));

        let mut target = base();
        target.target = "aarch64-apple-darwin".to_owned();
        assert_ne!(baseline, target.result_identity().expect("target identity"));

        let mut dependency = base();
        dependency.dependencies[0].result =
            ResultId::parse(format!("sha256:{}", "44".repeat(32))).expect("result id");
        assert_ne!(
            baseline,
            dependency.result_identity().expect("dependency identity")
        );

        let mut build_tool = base();
        build_tool.build_tools[0].version = "cmake 4.2".to_owned();
        assert_ne!(
            baseline,
            build_tool.result_identity().expect("build tool identity")
        );

        let mut configuration = base();
        configuration
            .configuration
            .insert("build_type".to_owned(), "debug".to_owned());
        assert_ne!(
            baseline,
            configuration.result_identity().expect("configuration identity")
        );
    }

    #[test]
    fn local_source_refuses_reusable_result_identity() {
        let mut input = base();
        input.source = LockedSource::Path {
            path: prep_manifest::LocalSourcePath::parse("../local").expect("path"),
        };
        assert_eq!(
            input.result_identity(),
            Err(IdentityError::NonCacheableSource)
        );
    }

    #[test]
    fn duplicate_tool_role_fails_closed() {
        let mut input = base();
        input.build_tools.push(ToolIdentity {
            role: "cmake".to_owned(),
            executable: "/opt/cmake/bin/cmake".to_owned(),
            version: "cmake 4.2".to_owned(),
        });
        assert!(matches!(
            input.result_identity(),
            Err(IdentityError::DuplicateToolRole(_))
        ));
    }
}
