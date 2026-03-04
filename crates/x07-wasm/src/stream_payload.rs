use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};

fn is_json_string_safe_bytes(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .all(|b| !matches!(*b, 0x00..=0x1f | b'"' | b'\\'))
}

pub fn stream_payload_to_bytes(payload: &Value) -> Result<Vec<u8>> {
    let bytes_len = payload
        .get("bytes_len")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    if let Some(b64) = payload.get("base64").and_then(Value::as_str) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .context("decode base64")?;
        if bytes.len() != bytes_len {
            anyhow::bail!("stream_payload.bytes_len mismatch (base64)");
        }
        return Ok(bytes);
    }
    if let Some(text) = payload.get("text").and_then(Value::as_str) {
        let bytes = text.as_bytes().to_vec();
        if bytes.len() != bytes_len {
            anyhow::bail!("stream_payload.bytes_len mismatch (text)");
        }
        return Ok(bytes);
    }
    if bytes_len == 0 {
        return Ok(Vec::new());
    }
    anyhow::bail!("stream_payload missing base64/text for non-empty body")
}

pub fn bytes_to_stream_payload(bytes: &[u8]) -> Value {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut obj = serde_json::Map::new();
    obj.insert("bytes_len".to_string(), json!(bytes.len() as u64));
    obj.insert("base64".to_string(), Value::String(b64));
    if is_json_string_safe_bytes(bytes) {
        if let Ok(text) = std::str::from_utf8(bytes) {
            obj.insert("text".to_string(), Value::String(text.to_string()));
        }
    }
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_stream_payload_includes_text_for_json_string_safe_utf8() {
        let v = bytes_to_stream_payload(b"hello");
        assert_eq!(v.get("bytes_len").and_then(Value::as_u64), Some(5));
        assert_eq!(v.get("text").and_then(Value::as_str), Some("hello"));
        assert!(v.get("base64").and_then(Value::as_str).is_some());
    }

    #[test]
    fn bytes_to_stream_payload_omits_text_for_quote() {
        let v = bytes_to_stream_payload(b"\"");
        assert_eq!(v.get("bytes_len").and_then(Value::as_u64), Some(1));
        assert!(v.get("text").is_none());
        assert!(v.get("base64").and_then(Value::as_str).is_some());
    }

    #[test]
    fn bytes_to_stream_payload_omits_text_for_backslash() {
        let v = bytes_to_stream_payload(b"\\");
        assert_eq!(v.get("bytes_len").and_then(Value::as_u64), Some(1));
        assert!(v.get("text").is_none());
        assert!(v.get("base64").and_then(Value::as_str).is_some());
    }

    #[test]
    fn bytes_to_stream_payload_omits_text_for_control_byte() {
        let v = bytes_to_stream_payload(b"\n");
        assert_eq!(v.get("bytes_len").and_then(Value::as_u64), Some(1));
        assert!(v.get("text").is_none());
        assert!(v.get("base64").and_then(Value::as_str).is_some());
    }

    #[test]
    fn bytes_to_stream_payload_omits_text_for_non_utf8() {
        let v = bytes_to_stream_payload(&[0xff]);
        assert_eq!(v.get("bytes_len").and_then(Value::as_u64), Some(1));
        assert!(v.get("text").is_none());
        assert!(v.get("base64").and_then(Value::as_str).is_some());
    }
}
