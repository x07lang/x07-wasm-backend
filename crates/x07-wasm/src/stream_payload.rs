use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};

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
    if let Ok(text) = std::str::from_utf8(bytes) {
        obj.insert("text".to_string(), Value::String(text.to_string()));
    }
    Value::Object(obj)
}

pub fn bytes_to_stream_payload_compact(bytes: &[u8]) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("bytes_len".to_string(), json!(bytes.len() as u64));
    if bytes.is_empty() {
        return Value::Object(obj);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        obj.insert("text".to_string(), Value::String(text.to_string()));
        return Value::Object(obj);
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    obj.insert("base64".to_string(), Value::String(b64));
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
    fn bytes_to_stream_payload_includes_text_for_json_bytes() {
        let v = bytes_to_stream_payload(br#"{"projects":[]}"#);
        assert_eq!(v.get("bytes_len").and_then(Value::as_u64), Some(15));
        assert_eq!(
            v.get("text").and_then(Value::as_str),
            Some(r#"{"projects":[]}"#)
        );
        assert!(v.get("base64").and_then(Value::as_str).is_some());
    }

    #[test]
    fn bytes_to_stream_payload_omits_text_for_non_utf8() {
        let v = bytes_to_stream_payload(&[0xff]);
        assert_eq!(v.get("bytes_len").and_then(Value::as_u64), Some(1));
        assert!(v.get("text").is_none());
        assert!(v.get("base64").and_then(Value::as_str).is_some());
    }

    #[test]
    fn bytes_to_stream_payload_compact_omits_base64_for_utf8() {
        let v = bytes_to_stream_payload_compact(b"hello");
        assert_eq!(v.get("bytes_len").and_then(Value::as_u64), Some(5));
        assert_eq!(v.get("text").and_then(Value::as_str), Some("hello"));
        assert!(v.get("base64").is_none());
        assert_eq!(stream_payload_to_bytes(&v).unwrap(), b"hello".to_vec());
    }

    #[test]
    fn bytes_to_stream_payload_compact_includes_base64_for_non_utf8() {
        let v = bytes_to_stream_payload_compact(&[0xff]);
        assert_eq!(v.get("bytes_len").and_then(Value::as_u64), Some(1));
        assert!(v.get("text").is_none());
        assert!(v.get("base64").and_then(Value::as_str).is_some());
        assert_eq!(stream_payload_to_bytes(&v).unwrap(), vec![0xff]);
    }
}
