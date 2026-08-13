use prep_manifest::{Lockfile, Manifest};

#[test]
fn manifest_source_unknown_fields_fail_closed() {
    let input = r#"
schema = "prep/1"

[package]
name = "hello"
version = "1"

[[dependencies]]
name = "fmt"
version = "1"

[dependencies.source]
kind = "git"
url = "https://example.invalid/fmt.git"
ref = "main"
unexpected = true
"#;

    assert!(Manifest::parse(input).is_err());
}

#[test]
fn locked_source_unknown_fields_fail_closed() {
    let input = r#"
schema = "prep.lock/1"

[root]
name = "hello"
version = "1"
dependencies = ["fmt"]

[[package]]
name = "fmt"
version = "1"
dependencies = []

[package.source]
kind = "git"
url = "https://example.invalid/fmt.git"
commit = "0123456789abcdef0123456789abcdef01234567"
unexpected = true

[package.source.provider]
name = "builtin.git"
version = "0.1.0"
"#;

    assert!(Lockfile::parse(input).is_err());
}
