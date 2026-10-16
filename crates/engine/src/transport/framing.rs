//! Length-prefixed JSON frames for the byte-stream transports (socket, stdio): a 4-byte big-endian length,
//! then that many bytes of UTF-8 JSON holding one object.

use serde_json::Value;

use crate::protocol::EngineError;

pub use bifurc_client::frame::MAX_FRAME_BYTES;
pub const FRAME_HEADER_BYTES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    Oversized(String),
    InvalidJson(String),
    NotAnObject,
}

impl FrameError {
    pub fn message(&self) -> String {
        match self {
            FrameError::Oversized(message) | FrameError::InvalidJson(message) => message.clone(),
            FrameError::NotAnObject => "Frame body is not a JSON object.".to_string(),
        }
    }
}

impl From<FrameError> for EngineError {
    fn from(err: FrameError) -> Self {
        EngineError::bad_request(err.message())
    }
}

pub fn encode_frame(message: &Value) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(message).map_err(|e| FrameError::InvalidJson(e.to_string()))?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversized(format!(
            "Frame of {} bytes exceeds MAX_FRAME_BYTES ({MAX_FRAME_BYTES}).",
            body.len()
        )));
    }
    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Reassembles frames from arbitrarily split chunks. After an error the stream is unusable: the caller
/// closes it.
#[derive(Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    failed: bool,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    /// Feed bytes; returns every frame they complete.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>, FrameError> {
        if self.failed {
            return Ok(Vec::new());
        }
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();
        let mut offset = 0;
        loop {
            let rest = &self.buffer[offset..];
            if rest.len() < FRAME_HEADER_BYTES {
                break;
            }
            let length = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
            if length > MAX_FRAME_BYTES {
                return Err(self.fail(FrameError::Oversized(format!(
                    "Frame declares {length} bytes, above the {MAX_FRAME_BYTES}-byte limit. Closing the stream."
                ))));
            }
            let end = FRAME_HEADER_BYTES + length;
            if rest.len() < end {
                break;
            }
            let parsed: Value = match serde_json::from_slice(&rest[FRAME_HEADER_BYTES..end]) {
                Ok(value) => value,
                Err(e) => {
                    return Err(self.fail(FrameError::InvalidJson(format!("Frame body is not valid JSON: {e}"))));
                }
            };
            if !parsed.is_object() {
                return Err(self.fail(FrameError::NotAnObject));
            }
            frames.push(parsed);
            offset += end;
        }
        self.buffer.drain(..offset);
        Ok(frames)
    }

    fn fail(&mut self, error: FrameError) -> FrameError {
        self.failed = true;
        self.buffer.clear();
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_survive_arbitrary_splits() {
        let mut bytes = encode_frame(&json!({ "id": "1" })).unwrap();
        bytes.extend(encode_frame(&json!({ "id": "2", "text": "héllo" })).unwrap());
        let mut decoder = FrameDecoder::new();
        let mut frames = Vec::new();
        for chunk in bytes.chunks(3) {
            frames.extend(decoder.push(chunk).unwrap());
        }
        assert_eq!(frames, vec![json!({ "id": "1" }), json!({ "id": "2", "text": "héllo" })]);
    }

    #[test]
    fn a_non_object_body_fails_the_stream() {
        let body = b"[1]";
        let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        let mut decoder = FrameDecoder::new();
        assert_eq!(decoder.push(&bytes), Err(FrameError::NotAnObject));
        assert!(decoder.failed());
    }

    #[test]
    fn an_oversized_declaration_fails_before_the_body_arrives() {
        let mut decoder = FrameDecoder::new();
        let header = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes();
        assert!(matches!(decoder.push(&header), Err(FrameError::Oversized(_))));
    }
}
