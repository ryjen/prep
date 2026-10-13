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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BuilderOperation {
    #[serde(rename = "configure")]
    Configure,
    #[serde(rename = "build")]
    Build,
    #[serde(rename = "test")]
    Test,
    #[serde(rename = "install_to_staging")]
    InstallToStaging,
}

impl fmt::Display for BuilderOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        PluginOperation::from(*self).fmt(formatter)
    }
}

impl From<BuilderOperation> for PluginOperation {
    fn from(value: BuilderOperation) -> Self {
        match value {
            BuilderOperation::Configure => Self::Configure,
            BuilderOperation::Build => Self::Build,
            BuilderOperation::Test => Self::Test,
            BuilderOperation::InstallToStaging => Self::InstallToStaging,
        }
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
pub struct BuilderRequestFrame {
    pub protocol: String,
    pub id: String,
    #[serde(rename = "type")]
    pub operation: BuilderOperation,
    pub package: PluginPackage,
    pub context: PluginExecutionContext,
    #[serde(default)]
    pub arguments: Vec<String>,
}

impl BuilderRequestFrame {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        operation: BuilderOperation,
        package: PluginPackage,
        context: PluginExecutionContext,
        arguments: Vec<String>,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            id: id.into(),
            operation,
            package,
            context,
            arguments,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProtocolErrorCode {
    #[serde(rename = "invalid_request")]
    InvalidRequest,
    #[serde(rename = "unsupported_operation")]
    UnsupportedOperation,
    #[serde(rename = "unsupported_build_system")]
    UnsupportedBuildSystem,
    #[serde(rename = "not_found")]
    NotFound,
    #[serde(rename = "unavailable")]
    Unavailable,
    #[serde(rename = "build_failed")]
    BuildFailed,
    #[serde(rename = "test_failed")]
    TestFailed,
    #[serde(rename = "install_failed")]
    InstallFailed,
    #[serde(rename = "policy_denied")]
    PolicyDenied,
    #[serde(rename = "cancelled")]
    Cancelled,
    #[serde(rename = "timeout")]
    Timeout,
    #[serde(rename = "internal")]
    Internal,
}

impl fmt::Display for ProtocolErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "invalid_request",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::UnsupportedBuildSystem => "unsupported_build_system",
            Self::NotFound => "not_found",
            Self::Unavailable => "unavailable",
            Self::BuildFailed => "build_failed",
            Self::TestFailed => "test_failed",
            Self::InstallFailed => "install_failed",
            Self::PolicyDenied => "policy_denied",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::Internal => "internal",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProtocolErrorBody {
    pub code: ProtocolErrorCode,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResultFrameType {
    #[serde(rename = "result")]
    Result,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatus {
    Ok,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResultFrame<T> {
    Ok {
        protocol: String,
        id: String,
        #[serde(rename = "type")]
        frame_type: ResultFrameType,
        value: T,
    },
    Error {
        protocol: String,
        id: String,
        #[serde(rename = "type")]
        frame_type: ResultFrameType,
        error: ProtocolErrorBody,
    },
}

impl<T> ResultFrame<T> {
    #[must_use]
    pub fn ok(id: impl Into<String>, value: T) -> Self {
        Self::Ok {
            protocol: PROTOCOL_V1.to_owned(),
            id: id.into(),
            frame_type: ResultFrameType::Result,
            value,
        }
    }

    #[must_use]
    pub fn error(id: impl Into<String>, error: ProtocolErrorBody) -> Self {
        Self::Error {
            protocol: PROTOCOL_V1.to_owned(),
            id: id.into(),
            frame_type: ResultFrameType::Result,
            error,
        }
    }

    #[must_use]
    pub fn protocol(&self) -> &str {
        match self {
            Self::Ok { protocol, .. } | Self::Error { protocol, .. } => protocol,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Ok { id, .. } | Self::Error { id, .. } => id,
        }
    }

    #[must_use]
    pub fn frame_type(&self) -> ResultFrameType {
        match self {
            Self::Ok { frame_type, .. } | Self::Error { frame_type, .. } => *frame_type,
        }
    }

    #[must_use]
    pub fn status(&self) -> ResultStatus {
        match self {
            Self::Ok { .. } => ResultStatus::Ok,
            Self::Error { .. } => ResultStatus::Error,
        }
    }

    #[must_use]
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Ok { value, .. } => Some(value),
            Self::Error { .. } => None,
        }
    }

    #[must_use]
    pub fn error_body(&self) -> Option<&ProtocolErrorBody> {
        match self {
            Self::Ok { .. } => None,
            Self::Error { error, .. } => Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProbeBuildSystemResult {
    pub supported: bool,
}

pub type ProbeBuildSystemResultFrame = ResultFrame<ProbeBuildSystemResult>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfigureResult {
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildResult {
    pub built: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TestResult {
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstallToStagingResult {
    pub installed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventFrameType {
    #[serde(rename = "event")]
    Event,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginEventKind {
    #[serde(rename = "progress")]
    Progress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressEventError {
    message: &'static str,
}

impl fmt::Display for ProgressEventError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl Error for ProgressEventError {}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProgressEventFrame {
    pub protocol: String,
    pub id: String,
    #[serde(rename = "type")]
    pub frame_type: EventFrameType,
    pub event: PluginEventKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProgressEventFrame {
    protocol: String,
    id: String,
    #[serde(rename = "type")]
    frame_type: EventFrameType,
    event: PluginEventKind,
    message: String,
    #[serde(default)]
    completed: Option<u64>,
    #[serde(default)]
    total: Option<u64>,
}

impl ProgressEventFrame {
    pub fn new(
        id: impl Into<String>,
        message: impl Into<String>,
        progress: Option<(u64, u64)>,
    ) -> Result<Self, ProgressEventError> {
        let (completed, total) = progress.map_or((None, None), |(completed, total)| {
            (Some(completed), Some(total))
        });
        Self::from_parts(
            PROTOCOL_V1.to_owned(),
            id.into(),
            EventFrameType::Event,
            PluginEventKind::Progress,
            message.into(),
            completed,
            total,
        )
    }

    fn from_parts(
        protocol: String,
        id: String,
        frame_type: EventFrameType,
        event: PluginEventKind,
        message: String,
        completed: Option<u64>,
        total: Option<u64>,
    ) -> Result<Self, ProgressEventError> {
        match (completed, total) {
            (None, None) => {}
            (Some(completed), Some(total)) if total > 0 && completed <= total => {}
            (Some(_), Some(0)) => {
                return Err(ProgressEventError {
                    message: "progress total must be greater than zero",
                });
            }
            (Some(_), Some(_)) => {
                return Err(ProgressEventError {
                    message: "progress completed value exceeds total",
                });
            }
            _ => {
                return Err(ProgressEventError {
                    message: "progress completed and total must be provided together",
                });
            }
        }

        Ok(Self {
            protocol,
            id,
            frame_type,
            event,
            message,
            completed,
            total,
        })
    }
}

impl<'de> Deserialize<'de> for ProgressEventFrame {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawProgressEventFrame::deserialize(deserializer)?;
        Self::from_parts(
            raw.protocol,
            raw.id,
            raw.frame_type,
            raw.event,
            raw.message,
            raw.completed,
            raw.total,
        )
        .map_err(serde::de::Error::custom)
    }
}


#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HostPlanningOperation {
    #[serde(rename = "probe_host_dependency")]
    ProbeHostDependency,
    #[serde(rename = "plan_host_change")]
    PlanHostChange,
}

impl From<HostPlanningOperation> for PluginOperation {
    fn from(operation: HostPlanningOperation) -> Self {
        match operation {
            HostPlanningOperation::ProbeHostDependency => Self::ProbeHostDependency,
            HostPlanningOperation::PlanHostChange => Self::PlanHostChange,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostDependency {
    pub manager: String,
    pub package: String,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostPlanningRequest {
    pub protocol: String,
    pub id: String,
    #[serde(rename = "type")]
    pub operation: HostPlanningOperation,
    pub dependency: HostDependency,
}

impl HostPlanningRequest {
    #[must_use]
    pub fn new(id: impl Into<String>, operation: HostPlanningOperation, dependency: HostDependency) -> Self {
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            id: id.into(),
            operation,
            dependency,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostProbeResult {
    pub present: bool,
    #[serde(default)]
    pub installed_version: Option<String>,
}

pub type HostProbeResultFrame = ResultFrame<HostProbeResult>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HostChangeKind {
    #[serde(rename = "install")]
    Install,
    #[serde(rename = "upgrade")]
    Upgrade,
    #[serde(rename = "remove")]
    Remove,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostChangePlan {
    pub provider: String,
    pub dependency: HostDependency,
    pub change: HostChangeKind,
    pub requires_privilege: bool,
    pub summary: String,
}

pub type HostChangePlanResultFrame = ResultFrame<HostChangePlan>;

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn host_planning_round_trips_without_executable_apply_operation() {
        let dependency = HostDependency {
            manager: "apt".into(),
            package: "libarchive-dev".into(),
            version: Some("1.0".into()),
        };
        let request = HostPlanningRequest::new(
            "plan-1",
            HostPlanningOperation::PlanHostChange,
            dependency.clone(),
        );
        let encoded = encode_frame(&request).unwrap();
        let decoded: HostPlanningRequest = decode_frame(&encoded, DEFAULT_MAX_FRAME_BYTES).unwrap();
        assert_eq!(decoded, request);
        let plan = HostChangePlan {
            provider: "system-apt".into(),
            dependency,
            change: HostChangeKind::Install,
            requires_privilege: true,
            summary: "Install native library".into(),
        };
        let frame = HostChangePlanResultFrame::ok("plan-1", plan);
        let encoded = encode_frame(&frame).unwrap();
        let decoded: HostChangePlanResultFrame = decode_frame(&encoded, DEFAULT_MAX_FRAME_BYTES).unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn host_planning_rejects_apply_unknown_and_execution_fields() {
        for operation in ["apply_host_change", "shell", "configure"] {
            let json = format!(
                r#"{{"protocol":"prep.plugin/1","id":"1","type":"{operation}","dependency":{{"manager":"apt","package":"example"}}}}"#
            );
            assert!(decode_frame::<HostPlanningRequest>(&json, DEFAULT_MAX_FRAME_BYTES).is_err());
        }
        let raw = r#"{"protocol":"prep.plugin/1","id":"1","type":"plan_host_change","dependency":{"manager":"apt","package":"example"},"command":"sudo apt install example"}"#;
        assert!(decode_frame::<HostPlanningRequest>(raw, DEFAULT_MAX_FRAME_BYTES).is_err());
        let plan = r#"{"provider":"apt","dependency":{"manager":"apt","package":"example"},"change":"install","requires_privilege":true,"summary":"install","argv":["sudo"]}"#;
        assert!(decode_frame::<HostChangePlan>(plan, DEFAULT_MAX_FRAME_BYTES).is_err());
    }

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
    fn builder_request_round_trips_with_builder_only_operation() {
        let request = BuilderRequestFrame::new(
            "42",
            BuilderOperation::Configure,
            PluginPackage {
                name: "hello".to_owned(),
                version: "1.0.0".to_owned(),
            },
            PluginExecutionContext {
                source_dir: "/source".to_owned(),
                build_dir: "/build".to_owned(),
                staging_dir: "/staging".to_owned(),
                dependency_prefixes: Vec::new(),
                environment: BTreeMap::new(),
                offline: false,
            },
            vec!["-DCMAKE_BUILD_TYPE=Release".to_owned()],
        );

        let encoded = encode_frame(&request).expect("builder request should serialize");
        let decoded: BuilderRequestFrame =
            decode_frame(&encoded, DEFAULT_MAX_FRAME_BYTES).expect("builder request should parse");
        assert_eq!(decoded, request);
        assert!(encoded.contains(r#""type":"configure""#));
    }

    #[test]
    fn builder_request_rejects_host_operation() {
        let input = r#"{"protocol":"prep.plugin/1","id":"42","type":"apply_host_change","package":{"name":"hello","version":"1.0.0"},"context":{"source_dir":"/source","build_dir":"/build","staging_dir":"/staging"},"arguments":[]}"#;
        let error = decode_frame::<BuilderRequestFrame>(input, DEFAULT_MAX_FRAME_BYTES)
            .expect_err("host operation must not decode as a builder request");
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
    fn typed_result_frame_rejects_unknown_status_and_error_code() {
        let unknown_status = r#"{"protocol":"prep.plugin/1","id":"42","type":"result","status":"maybe","value":{"supported":true}}"#;
        let error =
            decode_frame::<ProbeBuildSystemResultFrame>(unknown_status, DEFAULT_MAX_FRAME_BYTES)
                .expect_err("unknown result status should fail");
        assert!(matches!(error, CodecError::InvalidJson(_)));

        let unknown_code = r#"{"protocol":"prep.plugin/1","id":"42","type":"result","status":"error","error":{"code":"shell_failed","message":"nope","retryable":false}}"#;
        let error =
            decode_frame::<ProbeBuildSystemResultFrame>(unknown_code, DEFAULT_MAX_FRAME_BYTES)
                .expect_err("unknown error code should fail");
        assert!(matches!(error, CodecError::InvalidJson(_)));
    }

    #[test]
    fn typed_result_frame_rejects_mixed_success_and_error_payloads() {
        let input = r#"{"protocol":"prep.plugin/1","id":"42","type":"result","status":"ok","value":{"supported":true},"error":{"code":"internal","message":"mixed"}}"#;
        let error = decode_frame::<ProbeBuildSystemResultFrame>(input, DEFAULT_MAX_FRAME_BYTES)
            .expect_err("mixed result payload should fail");
        assert!(matches!(error, CodecError::InvalidJson(_)));
    }

    #[test]
    fn progress_event_round_trips_and_rejects_invalid_progress() {
        let frame = ProgressEventFrame::new("42", "building", Some((20, 100)))
            .expect("valid progress event");
        let encoded = encode_frame(&frame).expect("progress event should serialize");
        let decoded: ProgressEventFrame =
            decode_frame(&encoded, DEFAULT_MAX_FRAME_BYTES).expect("progress event should parse");
        assert_eq!(decoded, frame);

        let input = r#"{"protocol":"prep.plugin/1","id":"42","type":"event","event":"progress","message":"bad","completed":101,"total":100}"#;
        let error = decode_frame::<ProgressEventFrame>(input, DEFAULT_MAX_FRAME_BYTES)
            .expect_err("completed > total should fail");
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
