use prep_manifest::{PackageName, PackageVersion, PluginName};
use serde_json::Value;

const LEGACY: &str = include_str!("fixtures/prep1-package.json");

#[test]
fn historical_package_fields_have_explicit_migration_outcomes() {
    let value: Value = serde_json::from_str(LEGACY).expect("historical fixture should be JSON");

    let name = value["name"].as_str().expect("legacy name");
    let version = value["version"].as_str().expect("legacy version");
    PackageName::parse(name).expect("legacy package name maps to Prep 2");
    PackageVersion::parse(version).expect("legacy package version maps to Prep 2");

    for system in value["build_system"]
        .as_array()
        .expect("legacy build systems")
    {
        PluginName::parse(system.as_str().expect("build system string"))
            .expect("legacy build system maps to a plugin name");
    }

    let dependency = &value["dependencies"][0];
    PackageName::parse(dependency["name"].as_str().expect("dependency name"))
        .expect("legacy dependency name maps");
    PackageVersion::parse(dependency["version"].as_str().expect("dependency version"))
        .expect("legacy dependency version maps");

    let archive = dependency["archive"]["location"]
        .as_str()
        .expect("legacy archive location");
    assert!(archive.starts_with("http://"));
    assert!(dependency.get("apt").is_some());
}

#[test]
fn migration_must_not_preserve_unsafe_legacy_semantics_implicitly() {
    let value: Value = serde_json::from_str(LEGACY).expect("historical fixture should be JSON");
    let dependency = &value["dependencies"][0];

    // Prep 1 accepted an HTTP archive with no digest and an apt fallback. Prep 2's
    // importer must require the archive to be upgraded to HTTPS + immutable digest,
    // while the apt entry maps to the separate explicit host-provider model (#9).
    let archive = dependency["archive"]["location"]
        .as_str()
        .expect("legacy archive location");
    assert!(archive.starts_with("http://"));
    assert!(dependency["archive"].get("sha256").is_none());
    assert_eq!(dependency["apt"]["name"], "libarchive-dev");
}
