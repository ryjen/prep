use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const PROTOCOL_V1: &str = "prep.plugin/1";
pub const DEFAULT_MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub enum CodecError {
    EmptyFrame,
    FrameTooLarge { actual: usize, maximum: usize },
    InvalidJson(serde_json::Error),
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFrame => write!(formatter, "protocol frame is empty"),
            Self::FrameTooLarge { actual, maximum } => write!(
                formatter,
                "protocol frame is {actual} bytes; maximum is {maximum} bytes"
            ),
            Self::InvalidJson(error) => write!(formatter, "invalid protocol JSON: {error}"),
        }
    }
}

impl Error for CodecError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidJson(error) => Some(error),
            Self::EmptyFrame | Self::FrameTooLarge { .. } => None,
        }
    }
}

pub fn encode_frame<T: Serialize>(frame: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(frame)
}

pub fn decode_frame<T: DeserializeOwned>(line: &str, maximum: usize) -> Result<T, CodecError> {
    let frame = line.trim_end_matches(['\r', '\n']);
    if frame.is_empty() {
        return Err(CodecError::EmptyFrame);
    }
    if frame.len() > maximum {
        return Err(CodecError::FrameTooLarge {
            actual: frame.len(),
            maximum,
        });
    }
    serde_json::from_str(frame).map_err(CodecError::InvalidJson)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HelloFrame {
    pub protocol: String,
    pub id: String,
    #[serde(rename = "type")]
    pub frame_type: String,
    pub prep_version: String,
}

impl HelloFrame {
    #[must_use]
    pub fn new(id: impl Into<String>, prep_version: impl Into<String>) -> Self {
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            id: id.into(),
            frame_type: "hello".to_owned(),
            prep_version: prep_version.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PluginCapability {
    #[serde(rename = "network")]
    Network,
    #[serde(rename = "process.spawn")]
    ProcessSpawn,
    #[serde(rename = "filesystem.read.source")]
    FilesystemReadSource,
    #[serde(rename = "filesystem.write.staging")]
    FilesystemWriteStaging,
    #[serde(rename = "host.package_manager")]
    HostPackageManager,
    #[serde(rename = "privilege")]
    Privilege,
    #[serde(rename = "prompt")]
    Prompt,
    #[serde(rename = "prompt.secret")]
    PromptSecret,
}

impl fmt::Display for PluginCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Network => "network",
            Self::ProcessSpawn => "process.spawn",
            Self::FilesystemReadSource => "filesystem.read.source",
            Self::FilesystemWriteStaging => "filesystem.write.staging",
            Self::HostPackageManager => "host.package_manager",
            Self::Privilege => "privilege",
            Self::Prompt => "prompt",
            Self::PromptSecret => "prompt.secret",
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PluginOperation {
    #[serde(rename = "probe_build_system")]
    ProbeBuildSystem,
    #[serde(rename = "configure")]
    Configure,
    #[serde(rename = "build")]
    Build,
    #[serde(rename = "test")]
    Test,
    #[serde(rename = "install_to_staging")]
    InstallToStaging,
    #[serde(rename = "probe_host_dependency")]
    ProbeHostDependency,
    #[serde(rename = "plan_host_change")]
    PlanHostChange,
    #[serde(rename = "apply_host_change")]
    ApplyHostChange,
}

impl fmt::Display for PluginOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ProbeBuildSystem => "probe_build_system",
            Self::Configure => "configure",
            Self::Build => "build",
            Self::Test => "test",
            Self::InstallToStaging => "install_to_staging",
            Self::ProbeHostDependency => "probe_host_dependency",
            Self::PlanHostChange => "plan_host_change",
            Self::ApplyHostChange => "apply_host_change",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginPackage {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginExecutionContext {
    pub source_dir: String,
    pub build_dir: String,
    pub staging_dir: String,
    #[serde(default)]
    pub dependency_prefixes: Vec<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub offline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginDescriptor {
    pub name: String,
    pub version: String,
    pub operations: Vec<PluginOperation>,
    pub capabilities: Vec<PluginCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HelloResultFrame {
    pub protocol: String,
    pub id: String,
    #[serde(rename = "type")]
    pub frame_type: String,
    pub plugin: PluginDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProbeBuildSystemRequest {
    pub protocol: String,
    pub id: String,
    #[serde(rename = "type")]
    pub frame_type: PluginOperation,
    pub context: Value,
}

impl ProbeBuildSystemRequest {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            id: id.into(),
            frame_type: PluginOperation::ProbeBuildSystem,
            context: Value::Object(serde_json::Map::new()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProtocolErrorBody {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResultFrame {
    pub protocol: String,
    pub id: String,
    #[serde(rename = "type")]
    pub frame_type: String,
    pub status: String,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default)]
    pub error: Option<ProtocolErrorBody>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trips() {
        let frame = HelloFrame::new("1", "2.0.0-dev");
        let encoded = encode_frame(&frame).expect("hello frame should serialize");
        let decoded: HelloFrame =
            decode_frame(&encoded, DEFAULT_MAX_FRAME_BYTES).expect("hello frame should parse");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn oversized_frame_is_rejected_before_json_parsing() {
        let error = decode_frame::<Value>("12345", 4).expect_err("frame should be rejected");
        assert!(matches!(
            error,
            CodecError::FrameTooLarge {
                actual: 5,
                maximum: 4
            }
        ));
    }

    #[test]
    fn capability_round_trips_as_protocol_string() {
        let encoded =
            encode_frame(&PluginCapability::ProcessSpawn).expect("capability should serialize");
        assert_eq!(encoded, "\"process.spawn\"");

        let decoded: PluginCapability =
            decode_frame(&encoded, DEFAULT_MAX_FRAME_BYTES).expect("capability should parse");
        assert_eq!(decoded, PluginCapability::ProcessSpawn);
    }

    #[test]
    fn operation_round_trips_as_protocol_string() {
        let encoded =
            encode_frame(&PluginOperation::InstallToStaging).expect("operation should serialize");
        assert_eq!(encoded, "\"install_to_staging\"");

        let decoded: PluginOperation =
            decode_frame(&encoded, DEFAULT_MAX_FRAME_BYTES).expect("operation should parse");
        assert_eq!(decoded, PluginOperation::InstallToStaging);
    }

    #[test]
    fn unknown_operation_fails_closed() {
        let input = r#"{"protocol":"prep.plugin/1","id":"hello","type":"hello_result","plugin":{"name":"x","version":"1","operations":["shell"],"capabilities":[]}}"#;
        let error = decode_frame::<HelloResultFrame>(input, DEFAULT_MAX_FRAME_BYTES)
            .expect_err("unknown operation should be rejected");
        assert!(matches!(error, CodecError::InvalidJson(_)));
    }

    #[test]
    fn execution_context_round_trips_and_rejects_unknown_fields() {
        let context = PluginExecutionContext {
            source_dir: "/source".to_owned(),
            build_dir: "/build".to_owned(),
            staging_dir: "/staging".to_owned(),
            dependency_prefixes: vec!["/deps/a".to_owned()],
            environment: BTreeMap::from([("PATH".to_owned(), "/tools/bin".to_owned())]),
            offline: true,
        };
        let encoded = encode_frame(&context).expect("context should serialize");
        let decoded: PluginExecutionContext =
            decode_frame(&encoded, DEFAULT_MAX_FRAME_BYTES).expect("context should parse");
        assert_eq!(decoded, context);

        let input = r#"{"source_dir":"/source","build_dir":"/build","staging_dir":"/staging","unexpected":true}"#;
        let error = decode_frame::<PluginExecutionContext>(input, DEFAULT_MAX_FRAME_BYTES)
            .expect_err("unknown context field should be rejected");
        assert!(matches!(error, CodecError::InvalidJson(_)));
    }

    #[test]
    fn unknown_capability_fails_closed() {
        let input = r#"{"protocol":"prep.plugin/1","id":"hello","type":"hello_result","plugin":{"name":"x","version":"1","operations":["probe_build_system"],"capabilities":["filesystem.root"]}}"#;
        let error = decode_frame::<HelloResultFrame>(input, DEFAULT_MAX_FRAME_BYTES)
            .expect_err("unknown capability should be rejected");
        assert!(matches!(error, CodecError::InvalidJson(_)));
    }

    #[test]
    fn unknown_fields_fail_closed() {
        let input = r#"{"protocol":"prep.plugin/1","id":"1","type":"hello","prep_version":"x","extra":true}"#;
        let error = decode_frame::<HelloFrame>(input, DEFAULT_MAX_FRAME_BYTES)
            .expect_err("unknown fields should be rejected");
        assert!(matches!(error, CodecError::InvalidJson(_)));
    }
}
