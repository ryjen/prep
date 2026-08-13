use prep_protocol::{
    DEFAULT_MAX_FRAME_BYTES, HelloFrame, HelloResultFrame, PROTOCOL_V1, ProbeBuildSystemRequest,
    ResultFrame, decode_frame, encode_frame,
};
use std::error::Error;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

const HELLO_ID: &str = "hello";
const PROBE_ID: &str = "probe-build-system";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginProbe {
    pub name: String,
    pub version: String,
    pub supported: bool,
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
    ProcessFailed(Option<i32>),
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
            Self::ProcessFailed(code) => {
                write!(formatter, "plugin process failed with status {code:?}")
            }
        }
    }
}

impl Error for PluginProbeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn(error) | Self::Io { source: error, .. } => Some(error),
            _ => None,
        }
    }
}

pub fn probe_build_system(executable: &Path) -> Result<PluginProbe, PluginProbeError> {
    let mut child = Command::new(executable)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(PluginProbeError::Spawn)?;

    let mut stdin = child.stdin.take().ok_or_else(|| {
        PluginProbeError::Protocol("plugin stdin pipe was not created".to_owned())
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        PluginProbeError::Protocol("plugin stdout pipe was not created".to_owned())
    })?;
    let mut stdout = BufReader::new(stdout);

    let interaction = (|| {
        write_protocol_frame(
            &mut stdin,
            &HelloFrame::new(HELLO_ID, env!("CARGO_PKG_VERSION")),
        )?;
        let hello_line = read_bounded_line(&mut stdout)?;
        let hello: HelloResultFrame = decode_frame(&hello_line, DEFAULT_MAX_FRAME_BYTES)
            .map_err(|error| PluginProbeError::Protocol(error.to_string()))?;
        validate_handshake(&hello)?;

        write_protocol_frame(&mut stdin, &ProbeBuildSystemRequest::new(PROBE_ID))?;
        let result_line = read_bounded_line(&mut stdout)?;
        let result: ResultFrame = decode_frame(&result_line, DEFAULT_MAX_FRAME_BYTES)
            .map_err(|error| PluginProbeError::Protocol(error.to_string()))?;
        validate_result(&result)?;

        Ok::<_, PluginProbeError>((hello.plugin, result))
    })();

    drop(stdin);

    let (plugin, result) = match interaction {
        Ok(value) => value,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };

    let status = child.wait().map_err(|source| PluginProbeError::Io {
        operation: "wait",
        source,
    })?;
    if !status.success() {
        return Err(PluginProbeError::ProcessFailed(status.code()));
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
    })
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

fn read_bounded_line(reader: &mut impl BufRead) -> Result<String, PluginProbeError> {
    let mut line = String::new();
    let bytes_read = {
        let mut limited = reader.take((DEFAULT_MAX_FRAME_BYTES + 1) as u64);
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
    if bytes_read > DEFAULT_MAX_FRAME_BYTES {
        return Err(PluginProbeError::Protocol(format!(
            "protocol frame exceeds {DEFAULT_MAX_FRAME_BYTES} bytes"
        )));
    }
    if !line.ends_with('\n') {
        return Err(PluginProbeError::MissingNewline);
    }
    Ok(line)
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
}
