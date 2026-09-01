use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use thiserror::Error;

pub const PROTOCOL_VERSION: u32 = 2;
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreRequest {
    pub id: String,
    pub protocol_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    pub method: String,
    pub params: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreResponse {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CoreError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreEvent {
    pub sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    pub topic: String,
    pub payload: Value,
}

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame length {0} exceeds the 16 MiB limit")]
    Oversized(usize),
    #[error("frame was truncated")]
    Truncated,
    #[error("invalid JSON frame: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("protocol version {0} is not supported")]
    Version(u32),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn read_frame<T: for<'de> Deserialize<'de>>(
    reader: &mut impl Read,
) -> Result<Option<T>, FrameError> {
    let mut size = [0_u8; 4];
    let count = reader.read(&mut size)?;
    if count == 0 {
        return Ok(None);
    }
    if count != size.len() {
        reader
            .read_exact(&mut size[count..])
            .map_err(|_| FrameError::Truncated)?;
    }
    let length = u32::from_be_bytes(size) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::Oversized(length));
    }
    let mut payload = vec![0_u8; length];
    reader
        .read_exact(&mut payload)
        .map_err(|_| FrameError::Truncated)?;
    Ok(Some(serde_json::from_slice(&payload)?))
}

pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), FrameError> {
    let payload = serde_json::to_vec(value)?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversized(payload.len()));
    }
    writer.write_all(&(payload.len() as u32).to_be_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

/// One newline-delimited JSON line for streaming requests: hosts that opt
/// into `"stream": true` read events and the terminal response as NDJSON
/// instead of a single length-prefixed frame.
pub fn write_json_line<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), FrameError> {
    let payload = serde_json::to_vec(value)?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversized(payload.len()));
    }
    writer.write_all(&payload)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

impl CoreRequest {
    pub fn validate(&self) -> Result<(), FrameError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(FrameError::Version(self.protocol_version));
        }
        Ok(())
    }
}

impl CoreResponse {
    pub fn success(id: impl Into<String>, result: Value) -> Self {
        Self {
            id: id.into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(
        id: impl Into<String>,
        code: &str,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            id: id.into(),
            ok: false,
            result: None,
            error: Some(CoreError {
                code: code.into(),
                message: message.into(),
                retryable,
                details: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn round_trip<T>(fixture: &str, name: &str) -> T
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let value: T = serde_json::from_str(fixture)
            .unwrap_or_else(|error| panic!("fixture {name} does not parse: {error}"));
        let mut bytes = vec![];
        write_frame(&mut bytes, &value).unwrap();
        assert_eq!(
            read_frame::<T>(&mut Cursor::new(bytes)).unwrap().as_ref(),
            Some(&value),
            "fixture {name} does not round-trip across the frame boundary"
        );
        value
    }

    /// Every file in `protocol/fixtures/` must parse as its envelope type,
    /// round-trip across the frame boundary, and — for requests — use the
    /// current protocol version and an implemented core method. A fixture
    /// naming a method the service does not dispatch is dishonest
    /// documentation and fails here.
    #[test]
    fn every_fixture_round_trips_and_requests_use_implemented_methods() {
        let fixtures =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol/fixtures");
        let mut checked = 0_usize;
        for entry in std::fs::read_dir(fixtures).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let fixture = std::fs::read_to_string(&path).unwrap();
            if name.starts_with("request-") {
                let request: CoreRequest = round_trip(&fixture, &name);
                request
                    .validate()
                    .unwrap_or_else(|error| panic!("fixture {name} fails validation: {error}"));
                assert!(
                    crate::service::METHODS.contains(&request.method.as_str()),
                    "fixture {name} uses unimplemented method {}",
                    request.method
                );
            } else if name.starts_with("response-") {
                let _: CoreResponse = round_trip(&fixture, &name);
            } else if name.starts_with("event-") {
                let _: CoreEvent = round_trip(&fixture, &name);
            } else {
                panic!("fixture {name} has no request-/response-/event- prefix");
            }
            checked += 1;
        }
        assert!(checked >= 9, "the fixture corpus lost files: {checked}");
    }

    #[test]
    fn update_check_fixture_uses_the_current_protocol() {
        let fixture = include_str!("../../../protocol/fixtures/request-update-check.json");
        let request: CoreRequest = serde_json::from_str(fixture).unwrap();
        request.validate().unwrap();
        assert_eq!(request.method, "updates.check");
        assert!(request.workspace_id.is_none());
    }

    #[test]
    fn context_update_fixture_targets_a_workspace_with_structured_context() {
        let fixture =
            include_str!("../../../protocol/fixtures/request-workspace-context-update.json");
        let request: CoreRequest = serde_json::from_str(fixture).unwrap();
        request.validate().unwrap();
        assert_eq!(request.method, "workspace.context.update");
        assert!(request.workspace_id.is_some());
        let context: crate::local_state::ProjectContext =
            serde_json::from_value(request.params["context"].clone()).unwrap();
        assert_eq!(context.context_file_names, ["deck.pdf", "spec.md"]);
    }

    #[test]
    fn context_inspection_fixture_preserves_cross_platform_picker_paths() {
        let fixture = include_str!("../../../protocol/fixtures/request-context-files-inspect.json");
        let request: CoreRequest = serde_json::from_str(fixture).unwrap();
        request.validate().unwrap();
        assert_eq!(request.method, "context.files.inspect");
        let paths = request.params["paths"].as_array().unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].as_str().unwrap().starts_with('/'));
        assert!(paths[1].as_str().unwrap().starts_with("C:\\"));
    }

    #[test]
    fn progress_events_round_trip_as_single_ndjson_lines() {
        let fixture = include_str!("../../../protocol/fixtures/event-goals-progress.json");
        let event: CoreEvent = serde_json::from_str(fixture).unwrap();
        assert_eq!(event.topic, "goals.generate.progress");
        let mut bytes = vec![];
        write_json_line(&mut bytes, &event).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
        let reparsed: CoreEvent = serde_json::from_slice(&bytes[..bytes.len() - 1]).unwrap();
        assert_eq!(reparsed, event);
    }

    #[test]
    fn oversized_frames_are_rejected_before_allocation() {
        let mut bytes = ((MAX_FRAME_BYTES as u32) + 1).to_be_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        assert!(matches!(
            read_frame::<Value>(&mut Cursor::new(bytes)),
            Err(FrameError::Oversized(_))
        ));
    }

    #[test]
    fn unknown_protocol_versions_are_rejected() {
        let request = CoreRequest {
            id: "x".into(),
            protocol_version: 99,
            workspace_id: None,
            method: "system.ping".into(),
            params: Default::default(),
        };
        assert!(matches!(request.validate(), Err(FrameError::Version(99))));
    }
}
