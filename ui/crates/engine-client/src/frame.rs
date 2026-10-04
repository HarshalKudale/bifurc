//! The socket transport's framing: a 4-byte big-endian length, then that many bytes of UTF-8 JSON.
//!
//! Mirrors `packages/engine/src/transport/framing.ts`, including its 8 MiB ceiling. A frame over the ceiling
//! is a protocol error on the way in (the stream is no longer trustworthy) and a refusal on the way out.

use std::io::{self, Read, Write};

use serde_json::Value;

use crate::error::{ClientError, Result};

pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
const HEADER_BYTES: usize = 4;

pub fn encode_frame(message: &Value) -> Result<Vec<u8>> {
    let body = serde_json::to_vec(message)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(ClientError::Protocol(format!(
            "frame of {} bytes exceeds the {MAX_FRAME_BYTES}-byte limit",
            body.len()
        )));
    }
    let mut frame = Vec::with_capacity(HEADER_BYTES + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn write_frame(writer: &mut (impl Write + ?Sized), message: &Value) -> Result<()> {
    let frame = encode_frame(message)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

/// Reads one frame. `Ok(None)` is a clean end of stream between frames; EOF inside one is an error.
pub fn read_frame(reader: &mut impl Read) -> Result<Option<Value>> {
    let mut header = [0u8; HEADER_BYTES];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let length = u32::from_be_bytes(header) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(ClientError::Protocol(format!(
            "frame declares {length} bytes, above the {MAX_FRAME_BYTES}-byte limit"
        )));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|e| ClientError::Protocol(format!("frame body is not valid JSON: {e}")))?;
    if !value.is_object() {
        return Err(ClientError::Protocol("frame body is not a JSON object".to_owned()));
    }
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_frame_round_trips() {
        let message = json!({"id": "1", "action": "config.get", "payload": {}});
        let bytes = encode_frame(&message).unwrap();
        assert_eq!(&bytes[..4], &(bytes.len() as u32 - 4).to_be_bytes());
        assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), Some(message));
    }

    #[test]
    fn two_frames_in_one_buffer_are_read_in_order() {
        let mut bytes = encode_frame(&json!({"n": 1})).unwrap();
        bytes.extend(encode_frame(&json!({"n": 2})).unwrap());
        let mut reader = bytes.as_slice();
        assert_eq!(read_frame(&mut reader).unwrap(), Some(json!({"n": 1})));
        assert_eq!(read_frame(&mut reader).unwrap(), Some(json!({"n": 2})));
        assert_eq!(read_frame(&mut reader).unwrap(), None);
    }

    #[test]
    fn multibyte_text_is_measured_in_bytes() {
        let message = json!({"text": "héllo — ✓"});
        let bytes = encode_frame(&message).unwrap();
        assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), Some(message));
    }

    #[test]
    fn an_oversized_header_is_a_protocol_error_before_any_allocation() {
        let header = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes();
        let err = read_frame(&mut header.as_slice()).unwrap_err();
        assert!(matches!(err, ClientError::Protocol(_)));
    }

    #[test]
    fn a_body_cut_short_is_an_error_not_a_clean_end() {
        let mut bytes = encode_frame(&json!({"a": 1})).unwrap();
        bytes.pop();
        assert!(matches!(read_frame(&mut bytes.as_slice()), Err(ClientError::Io(_))));
    }

    #[test]
    fn a_non_object_body_is_refused() {
        let body = b"[1,2]";
        let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        assert!(matches!(read_frame(&mut bytes.as_slice()), Err(ClientError::Protocol(_))));
    }
}
