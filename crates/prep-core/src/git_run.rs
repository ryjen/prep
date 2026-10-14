//! Bounded execution for *read-only* Git reference resolution. A successful
//! response is only an object ID candidate; it does not prove commit type,
//! provenance or a safely materialized checkout.
use crate::git_command::{
    parse_exact_git_ref, prepare_exact_git_ref_resolution, prepare_locked_commit_kind_inspection,
};
use crate::plugin::{cleanup_remaining_process_group, configure_process_group, terminate_and_reap};
use prep_manifest::GitCommit;
use std::fmt;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

const MAX_REF_OUTPUT: usize = 4096;
const MAX_RUNTIME: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(10);
const TERMINATION_GRACE: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub enum GitResolutionError {
    InvalidPolicy,
    Io(io::Error),
    Timeout,
    OutputTooLarge,
    Unsuccessful,
    UnexpectedObjectType,
}
impl fmt::Display for GitResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy => f.write_str("invalid Git resolution process limits"),
            Self::Io(error) => write!(f, "Git source resolution failed: {error}"),
            Self::Timeout => f.write_str("Git source resolution exceeded its deadline"),
            Self::OutputTooLarge => f.write_str("Git source resolution response exceeds limit"),
            Self::Unsuccessful => f.write_str("Git source resolution process failed"),
            Self::UnexpectedObjectType => f.write_str("locked Git object is not a commit"),
        }
    }
}
impl std::error::Error for GitResolutionError {}
impl From<io::Error> for GitResolutionError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GitResolutionLimits {
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
}
impl Default for GitResolutionLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_stdout_bytes: MAX_REF_OUTPUT,
        }
    }
}

/// Run one sanitized read-only HTTPS ls-remote query with a deadline and
/// hard output bounds. The executable must be an operator-trusted Git binary.
/// The result is not yet eligible for source publication or build consumption.
pub fn resolve_git_reference(
    executable: &Path,
    remote: &str,
    full_ref: &str,
    limits: GitResolutionLimits,
) -> Result<GitCommit, GitResolutionError> {
    let command = prepare_exact_git_ref_resolution(executable, remote, full_ref)?;
    let output = capture_bounded(command, limits)?;
    parse_exact_git_ref(&output, full_ref).map_err(GitResolutionError::Io)
}

/// Inspect an already available, pinned Git object without fetching or
/// executing checkout operations. The repository path must be store-owned.
/// A valid result verifies only the Git object *kind*, not the tree contents
/// or its provenance, and grants no source/build publication authority.
pub fn inspect_git_commit_type(
    executable: &Path,
    repository: &Path,
    commit: &GitCommit,
    limits: GitResolutionLimits,
) -> Result<(), GitResolutionError> {
    let command = prepare_locked_commit_kind_inspection(executable, repository, commit);
    let output = capture_bounded(command, limits)?;
    verify_commit_type_output(&output)
}

fn verify_commit_type_output(output: &[u8]) -> Result<(), GitResolutionError> {
    if output == b"commit\n" {
        Ok(())
    } else {
        Err(GitResolutionError::UnexpectedObjectType)
    }
}

fn stdout_reader(
    mut stdout: impl Read + Send + 'static,
    maximum: usize,
) -> Receiver<io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .by_ref()
            .take(maximum as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    rx
}

fn terminate(child: &mut Child) {
    let id = child.id();
    let _ = terminate_and_reap(child, id, TERMINATION_GRACE);
    cleanup_remaining_process_group(id, TERMINATION_GRACE);
}

fn capture_bounded(
    mut command: Command,
    limits: GitResolutionLimits,
) -> Result<Vec<u8>, GitResolutionError> {
    if limits.timeout.is_zero()
        || limits.timeout > MAX_RUNTIME
        || limits.max_stdout_bytes == 0
        || limits.max_stdout_bytes > MAX_REF_OUTPUT
    {
        return Err(GitResolutionError::InvalidPolicy);
    }
    configure_process_group(&mut command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let id = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "missing Git stdout pipe"))?;
    let rx = stdout_reader(stdout, limits.max_stdout_bytes);
    let deadline = Instant::now() + limits.timeout;
    let mut received: Option<io::Result<Vec<u8>>> = None;
    loop {
        if received.is_none() {
            match rx.try_recv() {
                Ok(result) => received = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    terminate(&mut child);
                    return Err(GitResolutionError::Io(io::Error::from(
                        io::ErrorKind::BrokenPipe,
                    )));
                }
            }
        }
        if received.as_ref().is_some_and(|result| {
            result
                .as_ref()
                .is_ok_and(|bytes| bytes.len() > limits.max_stdout_bytes)
        }) {
            terminate(&mut child);
            return Err(GitResolutionError::OutputTooLarge);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if received.is_none() {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    received = match rx.recv_timeout(remaining) {
                        Ok(result) => Some(result),
                        Err(_) => {
                            cleanup_remaining_process_group(id, TERMINATION_GRACE);
                            return Err(GitResolutionError::Timeout);
                        }
                    };
                }
                cleanup_remaining_process_group(id, TERMINATION_GRACE);
                let bytes = received
                    .expect("reader returned")
                    .map_err(GitResolutionError::Io)?;
                if bytes.len() > limits.max_stdout_bytes {
                    return Err(GitResolutionError::OutputTooLarge);
                }
                if !status.success() {
                    return Err(GitResolutionError::Unsuccessful);
                }
                return Ok(bytes);
            }
            Ok(None) => {}
            Err(source) => {
                terminate(&mut child);
                return Err(GitResolutionError::Io(source));
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            terminate(&mut child);
            return Err(GitResolutionError::Timeout);
        }
        thread::sleep(POLL.min(remaining));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(script);
        command
    }

    #[test]
    fn commit_object_kind_must_match_the_exact_response() {
        assert!(verify_commit_type_output(b"commit\n").is_ok());
        for output in [
            &b"blob\n"[..],
            &b"tree\n"[..],
            &b"tag\n"[..],
            &b"commit"[..],
            &b"commit\r\n"[..],
            &b"commit\nextra"[..],
            &b""[..],
        ] {
            assert!(matches!(
                verify_commit_type_output(output),
                Err(GitResolutionError::UnexpectedObjectType)
            ));
        }
    }

    #[test]
    fn inspect_real_local_git_blob_fails_commit_kind_gate() {
        use std::io::Write;
        use std::process::Stdio;
        let directory = std::env::temp_dir().join(format!(
            "prep-git-type-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let init = Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg("-q")
            .arg(&directory)
            .status()
            .unwrap();
        assert!(init.success());
        let mut hash = Command::new("git")
            .arg("--git-dir")
            .arg(&directory)
            .arg("hash-object")
            .arg("-w")
            .arg("--stdin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        hash.stdin
            .take()
            .unwrap()
            .write_all(b"not a commit")
            .unwrap();
        let output = hash.wait_with_output().unwrap();
        assert!(output.status.success());
        let object = GitCommit::parse(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
        assert!(matches!(
            inspect_git_commit_type(
                Path::new("git"),
                &directory,
                &object,
                GitResolutionLimits::default()
            ),
            Err(GitResolutionError::UnexpectedObjectType)
        ));
        let tree = Command::new("git")
            .arg("-C")
            .arg(&directory)
            .arg("mktree")
            .output()
            .unwrap();
        assert!(tree.status.success());
        let tree_id = String::from_utf8(tree.stdout).unwrap();
        let commit = Command::new("git")
            .arg("-C")
            .arg(&directory)
            .arg("-c")
            .arg("user.name=Test")
            .arg("-c")
            .arg("user.email=test@example.invalid")
            .arg("commit-tree")
            .arg(tree_id.trim())
            .arg("-m")
            .arg("initial")
            .output()
            .unwrap();
        assert!(commit.status.success());
        let commit_id = GitCommit::parse(String::from_utf8(commit.stdout).unwrap().trim()).unwrap();
        assert!(
            inspect_git_commit_type(
                Path::new("git"),
                &directory,
                &commit_id,
                GitResolutionLimits::default()
            )
            .is_ok()
        );
        let nonexistent = GitCommit::parse("0".repeat(40)).unwrap();
        assert!(matches!(
            inspect_git_commit_type(
                Path::new("git"),
                &directory,
                &nonexistent,
                GitResolutionLimits::default()
            ),
            Err(GitResolutionError::Unsuccessful)
        ));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn captures_bounded_success_without_a_network_remote() {
        let out = capture_bounded(shell("printf 'abc'"), GitResolutionLimits::default()).unwrap();
        assert_eq!(out, b"abc");
    }

    #[test]
    fn fails_closed_when_output_is_larger_than_allowed() {
        let limits = GitResolutionLimits {
            max_stdout_bytes: 3,
            ..GitResolutionLimits::default()
        };
        assert!(matches!(
            capture_bounded(shell("printf 'abcd'"), limits),
            Err(GitResolutionError::OutputTooLarge)
        ));
    }

    #[test]
    fn times_out_and_reaps_hung_process() {
        let limits = GitResolutionLimits {
            timeout: Duration::from_millis(40),
            ..GitResolutionLimits::default()
        };
        assert!(matches!(
            capture_bounded(shell("sleep 2"), limits),
            Err(GitResolutionError::Timeout)
        ));
    }

    #[test]
    fn rejects_failed_exit_and_invalid_policy() {
        assert!(matches!(
            capture_bounded(shell("exit 7"), GitResolutionLimits::default()),
            Err(GitResolutionError::Unsuccessful)
        ));
        let limits = GitResolutionLimits {
            max_stdout_bytes: 4097,
            ..GitResolutionLimits::default()
        };
        assert!(matches!(
            capture_bounded(shell("exit 0"), limits),
            Err(GitResolutionError::InvalidPolicy)
        ));
    }

    #[test]
    fn rejects_invalid_remote_without_running_git() {
        assert!(matches!(
            resolve_git_reference(
                Path::new("/definitely/no/such/git"),
                "ssh://invalid.example/repo",
                "refs/heads/main",
                GitResolutionLimits::default()
            ),
            Err(GitResolutionError::Io(error)) if error.kind() == io::ErrorKind::InvalidInput
        ));
    }
}
