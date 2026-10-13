//! Git subprocess boundary. Not yet a source materializer.
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

#[cfg(test)]
mod tests {
    use super::*;
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
