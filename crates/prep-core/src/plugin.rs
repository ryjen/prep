use prep_protocol::{
    DEFAULT_MAX_FRAME_BYTES, HelloFrame, HelloResultFrame, PROTOCOL_V1, ProbeBuildSystemRequest,
    ResultFrame, decode_frame, encode_frame,
};
use std::error::Error;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const HELLO_ID: &str = "hello";
const PROBE_ID: &str = "probe-build-system";
const DEFAULT_MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDiagnostics {
    pub text: String,
    pub truncated: bool,
}

impl PluginDiagnostics {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginProcessPolicy {
    pub operation_timeout: Duration,
    pub termination_grace: Duration,
    pub max_frame_bytes: usize,
    pub max_diagnostic_bytes: usize,
}

impl Default for PluginProcessPolicy {
    fn default() -> Self {
        Self {
            operation_timeout: Duration::from_secs(30),
            termination_grace: Duration::from_millis(500),
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_diagnostic_bytes: DEFAULT_MAX_DIAGNOSTIC_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginPhase {
    Handshake,
    Result,
    Exit,
}

impl fmt::Display for PluginPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handshake => formatter.write_str("handshake"),
            Self::Result => formatter.write_str("result"),
            Self::Exit => formatter.write_str("process exit"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginProbe {
    pub name: String,
    pub version: String,
    pub supported: bool,
    pub diagnostics: PluginDiagnostics,
}

#[derive(Debug)]
pub enum PluginProbeError {
    Spawn(io::Error),
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Protocol(String),
    UnexpectedEof,
    MissingNewline,
    InvalidHandshake(String),
    InvalidResult(String),
    PluginReturnedError {
        code: String,
        message: String,
    },
    ProcessFailed {
        code: Option<i32>,
        diagnostics: PluginDiagnostics,
    },
    Timeout {
        phase: PluginPhase,
        timeout: Duration,
        diagnostics: PluginDiagnostics,
    },
    Cleanup {
        primary: Box<PluginProbeError>,
        message: String,
    },
}

impl fmt::Display for PluginProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "failed to spawn plugin: {error}"),
            Self::Io { operation, source } => {
                write!(formatter, "plugin {operation} failed: {source}")
            }
            Self::Protocol(message) => write!(formatter, "plugin protocol error: {message}"),
            Self::UnexpectedEof => {
                write!(formatter, "plugin closed stdout before a complete response")
            }
            Self::MissingNewline => write!(formatter, "plugin response was not newline-delimited"),
            Self::InvalidHandshake(message) => {
                write!(formatter, "invalid plugin handshake: {message}")
            }
            Self::InvalidResult(message) => write!(formatter, "invalid plugin result: {message}"),
            Self::PluginReturnedError { code, message } => {
                write!(formatter, "plugin returned {code}: {message}")
            }
            Self::ProcessFailed { code, .. } => {
                write!(formatter, "plugin process failed with status {code:?}")
            }
            Self::Timeout { phase, timeout, .. } => {
                write!(formatter, "plugin {phase} exceeded timeout {timeout:?}")
            }
            Self::Cleanup { primary, message } => {
                write!(
                    formatter,
                    "{primary}; plugin cleanup also failed: {message}"
                )
            }
        }
    }
}

impl Error for PluginProbeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn(error) | Self::Io { source: error, .. } => Some(error),
            Self::Cleanup { primary, .. } => Some(primary.as_ref()),
            _ => None,
        }
    }
}

pub fn probe_build_system(executable: &Path) -> Result<PluginProbe, PluginProbeError> {
    probe_build_system_with_policy(executable, PluginProcessPolicy::default())
}

pub fn probe_build_system_with_policy(
    executable: &Path,
    policy: PluginProcessPolicy,
) -> Result<PluginProbe, PluginProbeError> {
    let mut command = Command::new(executable);
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut command);

    let mut child = command.spawn().map_err(PluginProbeError::Spawn)?;
    let process_group_id = child.id();
    let mut stdin = child.stdin.take().ok_or_else(|| {
        PluginProbeError::Protocol("plugin stdin pipe was not created".to_owned())
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        PluginProbeError::Protocol("plugin stdout pipe was not created".to_owned())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        PluginProbeError::Protocol("plugin stderr pipe was not created".to_owned())
    })?;

    let stdout = spawn_stdout_reader(stdout, policy.max_frame_bytes);
    let diagnostics = DiagnosticCapture::new(stderr, policy.max_diagnostic_bytes);
    let deadline = Instant::now() + policy.operation_timeout;

    let interaction = (|| {
        write_protocol_frame(
            &mut stdin,
            &HelloFrame::new(HELLO_ID, env!("CARGO_PKG_VERSION")),
        )?;
        let hello_line = receive_protocol_line(&stdout, deadline, PluginPhase::Handshake)?;
        let hello: HelloResultFrame = decode_frame(&hello_line, policy.max_frame_bytes)
            .map_err(|error| PluginProbeError::Protocol(error.to_string()))?;
        validate_handshake(&hello)?;

        write_protocol_frame(&mut stdin, &ProbeBuildSystemRequest::new(PROBE_ID))?;
        let result_line = receive_protocol_line(&stdout, deadline, PluginPhase::Result)?;
        let result: ResultFrame = decode_frame(&result_line, policy.max_frame_bytes)
            .map_err(|error| PluginProbeError::Protocol(error.to_string()))?;
        validate_result(&result)?;

        Ok::<_, LifecycleFailure>((hello.plugin, result))
    })();

    drop(stdin);

    let (plugin, result) = match interaction {
        Ok(value) => value,
        Err(failure) => {
            return Err(cleanup_failure(
                &mut child,
                process_group_id,
                failure,
                &diagnostics,
                policy,
            ));
        }
    };

    let status = match wait_for_exit(&mut child, deadline) {
        Ok(status) => status,
        Err(failure) => {
            return Err(cleanup_failure(
                &mut child,
                process_group_id,
                failure,
                &diagnostics,
                policy,
            ));
        }
    };

    cleanup_remaining_process_group(process_group_id, policy.termination_grace);
    diagnostics.finish(policy.termination_grace);
    let diagnostics = diagnostics.snapshot();

    if !status.success() {
        return Err(PluginProbeError::ProcessFailed {
            code: status.code(),
            diagnostics,
        });
    }

    if result.status == "error" {
        let error = result.error.ok_or_else(|| {
            PluginProbeError::InvalidResult("error status omitted error details".to_owned())
        })?;
        return Err(PluginProbeError::PluginReturnedError {
            code: error.code,
            message: error.message,
        });
    }

    let supported = result
        .value
        .as_ref()
        .and_then(|value| value.get("supported"))
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| {
            PluginProbeError::InvalidResult(
                "successful probe result must contain boolean value.supported".to_owned(),
            )
        })?;

    Ok(PluginProbe {
        name: plugin.name,
        version: plugin.version,
        supported,
        diagnostics,
    })
}

#[derive(Debug)]
enum LifecycleFailure {
    Probe(PluginProbeError),
    Timeout(PluginPhase),
}

impl From<PluginProbeError> for LifecycleFailure {
    fn from(error: PluginProbeError) -> Self {
        Self::Probe(error)
    }
}

fn cleanup_failure(
    child: &mut Child,
    process_group_id: u32,
    failure: LifecycleFailure,
    diagnostics: &DiagnosticCapture,
    policy: PluginProcessPolicy,
) -> PluginProbeError {
    let cleanup = terminate_and_reap(child, process_group_id, policy.termination_grace);
    diagnostics.finish(policy.termination_grace);
    let captured = diagnostics.snapshot();

    let primary = match failure {
        LifecycleFailure::Probe(error) => error,
        LifecycleFailure::Timeout(phase) => PluginProbeError::Timeout {
            phase,
            timeout: policy.operation_timeout,
            diagnostics: captured,
        },
    };

    match cleanup {
        Ok(()) => primary,
        Err(error) => PluginProbeError::Cleanup {
            primary: Box::new(primary),
            message: error.to_string(),
        },
    }
}

fn receive_protocol_line(
    receiver: &Receiver<Result<String, PluginProbeError>>,
    deadline: Instant,
    phase: PluginPhase,
) -> Result<String, LifecycleFailure> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(LifecycleFailure::Timeout(phase));
    }

    match receiver.recv_timeout(remaining) {
        Ok(result) => result.map_err(LifecycleFailure::Probe),
        Err(RecvTimeoutError::Timeout) => Err(LifecycleFailure::Timeout(phase)),
        Err(RecvTimeoutError::Disconnected) => {
            Err(LifecycleFailure::Probe(PluginProbeError::UnexpectedEof))
        }
    }
}

fn wait_for_exit(child: &mut Child, deadline: Instant) -> Result<ExitStatus, LifecycleFailure> {
    loop {
        match child.try_wait().map_err(|source| {
            LifecycleFailure::Probe(PluginProbeError::Io {
                operation: "wait",
                source,
            })
        })? {
            Some(status) => return Ok(status),
            None => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(LifecycleFailure::Timeout(PluginPhase::Exit));
                }
                thread::sleep(WAIT_POLL_INTERVAL.min(remaining));
            }
        }
    }
}

fn spawn_stdout_reader(
    stdout: ChildStdout,
    max_frame_bytes: usize,
) -> Receiver<Result<String, PluginProbeError>> {
    let (sender, receiver) = mpsc::sync_channel(2);
    thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        loop {
            let result = read_bounded_line(&mut stdout, max_frame_bytes);
            let terminal = result.is_err();
            if sender.send(result).is_err() || terminal {
                break;
            }
        }
    });
    receiver
}

#[derive(Debug, Default)]
struct DiagnosticBuffer {
    bytes: Vec<u8>,
    truncated: bool,
}

impl DiagnosticBuffer {
    fn append(&mut self, bytes: &[u8], limit: usize) {
        let remaining = limit.saturating_sub(self.bytes.len());
        let accepted = remaining.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..accepted]);
        if accepted < bytes.len() {
            self.truncated = true;
        }
    }

    fn snapshot(&self) -> PluginDiagnostics {
        PluginDiagnostics {
            text: String::from_utf8_lossy(&self.bytes).into_owned(),
            truncated: self.truncated,
        }
    }
}

struct DiagnosticCapture {
    shared: Arc<Mutex<DiagnosticBuffer>>,
    done: Receiver<()>,
}

impl DiagnosticCapture {
    fn new(mut stderr: ChildStderr, limit: usize) -> Self {
        let shared = Arc::new(Mutex::new(DiagnosticBuffer::default()));
        let worker_shared = Arc::clone(&shared);
        let (done_sender, done) = mpsc::sync_channel(1);

        thread::spawn(move || {
            let mut chunk = [0_u8; 4096];
            loop {
                match stderr.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => worker_shared
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .append(&chunk[..count], limit),
                    Err(_) => break,
                }
            }
            let _ = done_sender.send(());
        });

        Self { shared, done }
    }

    fn finish(&self, wait: Duration) {
        let _ = self.done.recv_timeout(wait);
    }

    fn snapshot(&self) -> PluginDiagnostics {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .snapshot()
    }
}

fn validate_handshake(hello: &HelloResultFrame) -> Result<(), PluginProbeError> {
    if hello.protocol != PROTOCOL_V1 {
        return Err(PluginProbeError::InvalidHandshake(format!(
            "expected protocol {PROTOCOL_V1}, got {}",
            hello.protocol
        )));
    }
    if hello.id != HELLO_ID || hello.frame_type != "hello_result" {
        return Err(PluginProbeError::InvalidHandshake(
            "unexpected handshake id/type".to_owned(),
        ));
    }
    if hello.plugin.name.is_empty() || hello.plugin.version.is_empty() {
        return Err(PluginProbeError::InvalidHandshake(
            "plugin name/version must be non-empty".to_owned(),
        ));
    }
    if !hello
        .plugin
        .operations
        .iter()
        .any(|operation| operation == "probe_build_system")
    {
        return Err(PluginProbeError::InvalidHandshake(
            "plugin did not declare probe_build_system".to_owned(),
        ));
    }
    Ok(())
}

fn validate_result(result: &ResultFrame) -> Result<(), PluginProbeError> {
    if result.protocol != PROTOCOL_V1 {
        return Err(PluginProbeError::InvalidResult(format!(
            "expected protocol {PROTOCOL_V1}, got {}",
            result.protocol
        )));
    }
    if result.id != PROBE_ID || result.frame_type != "result" {
        return Err(PluginProbeError::InvalidResult(
            "unexpected result id/type".to_owned(),
        ));
    }

    match result.status.as_str() {
        "ok" if result.error.is_none() => Ok(()),
        "ok" => Err(PluginProbeError::InvalidResult(
            "ok status must not contain error details".to_owned(),
        )),
        "error" if result.error.is_some() => Ok(()),
        "error" => Err(PluginProbeError::InvalidResult(
            "error status must contain error details".to_owned(),
        )),
        status => Err(PluginProbeError::InvalidResult(format!(
            "unknown result status {status}"
        ))),
    }
}

fn write_protocol_frame<T: serde::Serialize>(
    writer: &mut impl Write,
    frame: &T,
) -> Result<(), PluginProbeError> {
    let encoded =
        encode_frame(frame).map_err(|error| PluginProbeError::Protocol(error.to_string()))?;
    writer
        .write_all(encoded.as_bytes())
        .and_then(|()| writer.write_all(b"\n"))
        .and_then(|()| writer.flush())
        .map_err(|source| PluginProbeError::Io {
            operation: "write",
            source,
        })
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    max_frame_bytes: usize,
) -> Result<String, PluginProbeError> {
    let mut line = String::new();
    let bytes_read = {
        let mut limited = reader.take((max_frame_bytes.saturating_add(1)) as u64);
        limited
            .read_line(&mut line)
            .map_err(|source| PluginProbeError::Io {
                operation: "read",
                source,
            })?
    };

    if bytes_read == 0 {
        return Err(PluginProbeError::UnexpectedEof);
    }
    if bytes_read > max_frame_bytes {
        return Err(PluginProbeError::Protocol(format!(
            "protocol frame exceeds {max_frame_bytes} bytes"
        )));
    }
    if !line.ends_with('\n') {
        return Err(PluginProbeError::MissingNewline);
    }
    Ok(line)
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

fn terminate_and_reap(child: &mut Child, process_group_id: u32, grace: Duration) -> io::Result<()> {
    #[cfg(unix)]
    {
        let term_sent = signal_process_group(process_group_id, "-TERM").unwrap_or(false);
        if !term_sent && child.try_wait()?.is_none() {
            child.kill()?;
        }

        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            let _ = child.try_wait()?;
            if !process_group_exists(process_group_id).unwrap_or(true) {
                break;
            }
            thread::sleep(
                WAIT_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
            );
        }

        if process_group_exists(process_group_id).unwrap_or(true) {
            let _ = signal_process_group(process_group_id, "-KILL");
        }
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        let _ = child.wait()?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        let _ = child.wait()?;
        Ok(())
    }
}

#[cfg(unix)]
fn cleanup_remaining_process_group(process_group_id: u32, grace: Duration) {
    if !process_group_exists(process_group_id).unwrap_or(false) {
        return;
    }

    let _ = signal_process_group(process_group_id, "-TERM");
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if !process_group_exists(process_group_id).unwrap_or(true) {
            return;
        }
        thread::sleep(WAIT_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
    let _ = signal_process_group(process_group_id, "-KILL");
}

#[cfg(not(unix))]
fn cleanup_remaining_process_group(_process_group_id: u32, _grace: Duration) {}

#[cfg(unix)]
fn signal_process_group(process_group_id: u32, signal: &str) -> io::Result<bool> {
    let status = Command::new("/bin/kill")
        .arg(signal)
        .arg(format!("-{process_group_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

#[cfg(unix)]
fn process_group_exists(process_group_id: u32) -> io::Result<bool> {
    signal_process_group(process_group_id, "-0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_plugin_has_typed_spawn_failure() {
        let error = probe_build_system(Path::new("/definitely/not/a/prep/plugin"))
            .expect_err("missing plugin should fail");
        assert!(matches!(error, PluginProbeError::Spawn(_)));
    }

    #[test]
    fn mixed_success_and_error_payload_is_rejected() {
        let result = ResultFrame {
            protocol: PROTOCOL_V1.to_owned(),
            id: PROBE_ID.to_owned(),
            frame_type: "result".to_owned(),
            status: "ok".to_owned(),
            value: None,
            error: Some(prep_protocol::ProtocolErrorBody {
                code: "internal".to_owned(),
                message: "should not coexist with ok".to_owned(),
                retryable: false,
            }),
        };

        assert!(matches!(
            validate_result(&result),
            Err(PluginProbeError::InvalidResult(_))
        ));
    }

    #[test]
    fn diagnostic_buffer_is_bounded() {
        let mut buffer = DiagnosticBuffer::default();
        buffer.append(b"abcdef", 4);
        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.text, "abcd");
        assert!(snapshot.truncated);
    }
}
