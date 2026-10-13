//! Git subprocess boundary. Not yet a source materializer.
use prep_manifest::GitCommit;
use std::path::Path;
use std::process::Command;

/// Construct Git without inherited user/system configuration or helpers.
pub fn isolated_git_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("filter.lfs.required=false")
        .arg("-c")
        .arg("filter.lfs.smudge=")
        .arg("-c")
        .arg("filter.lfs.process=")
        .arg("-c")
        .arg("protocol.file.allow=never")
        .arg("-c")
        .arg("submodule.recurse=false")
        .arg("-c")
        .arg("core.symlinks=false");
    command
}

/// Inspect a locked object ID, never an arbitrary reference or option.
/// Caller must bound process duration and output before using this in production.
pub fn prepare_locked_tree_inspection(
    executable: &Path,
    repository: &Path,
    revision: &str,
) -> std::io::Result<Command> {
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid pinned commit",
        ));
    }
    let mut command = isolated_git_command(executable);
    command
        .arg("-C")
        .arg(repository)
        .arg("ls-tree")
        .arg("-r")
        .arg("-z")
        .arg("--full-tree")
        .arg(revision);
    Ok(command)
}

/// Parse exact, bounded output from Git ls-remote without trusting fuzzy ref matches.
/// For annotated tags the peeled commit is preferred to the tag object.
/// The caller must still verify the resolved object is a commit before locking it.
pub fn parse_exact_git_ref(output: &[u8], requested_ref: &str) -> std::io::Result<GitCommit> {
    if !valid_full_git_ref(requested_ref) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Git reference must be an exact branch or tag",
        ));
    }
    if output.is_empty() || output.len() > 4096 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unexpected Git reference response size",
        ));
    }
    let response = std::str::from_utf8(output)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
    let mut direct = None;
    let mut peeled = None;
    let peeled_ref = format!("{requested_ref}^{{}}");
    for line in response.lines() {
        let (object, reference) = line
            .split_once('\t')
            .ok_or(std::io::ErrorKind::InvalidData)?;
        let commit = GitCommit::parse(object)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
        if reference == requested_ref {
            if direct.replace(commit).is_some() {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
        } else if requested_ref.starts_with("refs/tags/") && reference == peeled_ref {
            if peeled.replace(commit).is_some() {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
        } else {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    }
    if requested_ref.starts_with("refs/heads/") && peeled.is_some() {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    direct
        .ok_or(std::io::ErrorKind::NotFound.into())
        .map(|value| peeled.unwrap_or(value))
}

fn valid_full_git_ref(reference: &str) -> bool {
    let short = reference
        .strip_prefix("refs/heads/")
        .or_else(|| reference.strip_prefix("refs/tags/"));
    let Some(short) = short else {
        return false;
    };
    if short.is_empty() || reference.len() > 256 || short.contains("..") || short.ends_with(".lock")
    {
        return false;
    }
    short.split('/').all(|component| {
        !component.is_empty()
            && !component.starts_with('.')
            && !component.ends_with('.')
            && component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    })
}

/// Build an isolated, non-executing command to query one full reference.
/// Restrict URLs to HTTPS for v1; never invoke SSH or user-configured helpers.
pub fn prepare_exact_git_ref_resolution(
    executable: &Path,
    https_remote: &str,
    full_ref: &str,
) -> std::io::Result<Command> {
    if !https_remote.starts_with("https://")
        || https_remote.len() > 2048
        || https_remote
            .bytes()
            .any(|byte| byte <= b' ' || byte == 0x7f)
        || https_remote.contains('@')
        || !valid_full_git_ref(full_ref)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "untrusted Git remote or mutable/ambiguous ref",
        ));
    }
    let mut command = isolated_git_command(executable);
    command.arg("ls-remote").arg(https_remote).arg(full_ref);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ref_parser_requires_exact_reference_and_peeled_annotated_tags() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let good = format!("{a}\trefs/heads/main\n");
        assert_eq!(
            parse_exact_git_ref(good.as_bytes(), "refs/heads/main")
                .unwrap()
                .as_str(),
            a
        );
        assert!(parse_exact_git_ref(good.as_bytes(), "refs/heads/other").is_err());
        let tag = format!("{a}\trefs/tags/v1\n{b}\trefs/tags/v1^{{}}\n");
        assert_eq!(
            parse_exact_git_ref(tag.as_bytes(), "refs/tags/v1")
                .unwrap()
                .as_str(),
            b
        );
        assert!(parse_exact_git_ref(tag.as_bytes(), "refs/heads/v1").is_err());
        let ambiguous = format!("{a}\trefs/heads/main\n{b}\trefs/heads/main\n");
        assert!(parse_exact_git_ref(ambiguous.as_bytes(), "refs/heads/main").is_err());
        assert!(parse_exact_git_ref(&vec![b'a'; 4097], "refs/heads/main").is_err());
    }

    #[test]
    fn ref_resolution_command_denies_ssh_inline_credentials_and_patterns() {
        for remote in [
            "ssh://git.example/repo",
            "https://person@example.test/repo",
            "https://example.test/repo with spaces",
        ] {
            assert!(
                prepare_exact_git_ref_resolution(
                    Path::new("/does-not-need-to-exist"),
                    remote,
                    "refs/heads/main"
                )
                .is_err()
            );
        }
        for reference in [
            "HEAD",
            "main",
            "refs/heads/*",
            "refs/heads/../escape",
            "refs/tags/a.lock",
        ] {
            assert!(
                prepare_exact_git_ref_resolution(
                    Path::new("/does-not-need-to-exist"),
                    "https://example.test/repo",
                    reference
                )
                .is_err()
            );
        }
        assert!(
            prepare_exact_git_ref_resolution(
                Path::new("/does-not-need-to-exist"),
                "https://example.test/repo.git",
                "refs/heads/main"
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_unpinned_revision_before_spawning() {
        for bad in ["HEAD", "main", "--upload-pack=/bin/sh", "", "aabb"] {
            assert_eq!(
                prepare_locked_tree_inspection(Path::new("/nonexistent/git"), Path::new("."), bad)
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
    }
}
