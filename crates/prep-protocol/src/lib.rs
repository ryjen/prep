use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    let frame = line.trim_end_matches(|character| character == '\r' || character == '\n');
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginDescriptor {
    pub name: String,
    pub version: String,
    pub operations: Vec<String>,
    pub capabilities: Vec<String>,
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
    pub frame_type: String,
    pub context: Value,
}

impl ProbeBuildSystemRequest {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            protocol: PROTOCOL_V1.to_owned(),
            id: id.into(),
            frame_type: "probe_build_system".to_owned(),
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
    fn unknown_fields_fail_closed() {
        let input = r#"{"protocol":"prep.plugin/1","id":"1","type":"hello","prep_version":"x","extra":true}"#;
        let error = decode_frame::<HelloFrame>(input, DEFAULT_MAX_FRAME_BYTES)
            .expect_err("unknown fields should be rejected");
        assert!(matches!(error, CodecError::InvalidJson(_)));
    }
}
