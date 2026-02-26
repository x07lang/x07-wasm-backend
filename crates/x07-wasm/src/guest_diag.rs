use serde_json::Value;

use crate::diag::{Diagnostic, Severity, Stage};

#[derive(Debug, Clone)]
pub struct GuestDiag {
    pub code: String,
    pub data_obj: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct HostDiagError {
    pub code: &'static str,
    pub message: String,
}

impl HostDiagError {
    pub fn into_diagnostic(self) -> Diagnostic {
        Diagnostic::new(self.code, Severity::Error, Stage::Run, self.message)
    }
}

const HTTP_HEADER_DIAG_CODE: &str = "x-x07-diag-code";
const HTTP_HEADER_DIAG_DATA_B64: &str = "x-x07-diag-data-b64";

const CLI_SENTINEL_DIAG_CODE: &[u8] = b"x07-diag-code: ";
const CLI_SENTINEL_DIAG_DATA_B64: &[u8] = b"x07-diag-data-b64: ";

const MAX_SCAN_BYTES: usize = 8192;
const MAX_DATA_B64_BYTES: usize = 8192;
const MAX_DATA_DECODED_BYTES: usize = 4096;

fn trim_ows(bytes: &[u8]) -> &[u8] {
    // RFC 9110 OWS is SP/HTAB. Phase 4 pins to SP/HTAB trimming only.
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && (bytes[start] == b' ' || bytes[start] == b'\t') {
        start += 1;
    }
    while end > start && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t') {
        end -= 1;
    }
    &bytes[start..end]
}

fn parse_diag_code(trimmed: &[u8], invalid_code: &'static str) -> Result<String, HostDiagError> {
    if trimmed.is_empty() || trimmed.len() > 128 {
        return Err(HostDiagError {
            code: invalid_code,
            message: "diag code invalid length".to_string(),
        });
    }
    for &b in trimmed {
        if !b.is_ascii_uppercase() && !b.is_ascii_digit() && b != b'_' {
            return Err(HostDiagError {
                code: invalid_code,
                message: "diag code contains invalid byte".to_string(),
            });
        }
    }
    // Safety: bytes are ASCII.
    Ok(std::str::from_utf8(trimmed).unwrap().to_string())
}

fn b64_val(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(26 + (b - b'a')),
        b'0'..=b'9' => Some(52 + (b - b'0')),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn strict_base64_decode(
    trimmed: &[u8],
    invalid_code: &'static str,
) -> Result<Vec<u8>, HostDiagError> {
    if trimmed.len() > MAX_DATA_B64_BYTES {
        return Err(HostDiagError {
            code: invalid_code,
            message: "diag data base64 too long".to_string(),
        });
    }
    if !trimmed.len().is_multiple_of(4) {
        return Err(HostDiagError {
            code: invalid_code,
            message: "diag data base64 length not multiple of 4".to_string(),
        });
    }
    let mut out: Vec<u8> = Vec::with_capacity(trimmed.len() / 4 * 3);
    let blocks = trimmed.len() / 4;
    for bi in 0..blocks {
        let i = bi * 4;
        let c0 = trimmed[i];
        let c1 = trimmed[i + 1];
        let c2 = trimmed[i + 2];
        let c3 = trimmed[i + 3];

        let v0 = b64_val(c0).ok_or_else(|| HostDiagError {
            code: invalid_code,
            message: "diag data base64 invalid char".to_string(),
        })? as u32;
        let v1 = b64_val(c1).ok_or_else(|| HostDiagError {
            code: invalid_code,
            message: "diag data base64 invalid char".to_string(),
        })? as u32;

        let is_last = bi + 1 == blocks;
        let pad2 = c2 == b'=';
        let pad3 = c3 == b'=';

        if (pad2 || pad3) && !is_last {
            return Err(HostDiagError {
                code: invalid_code,
                message: "diag data base64 has padding before final block".to_string(),
            });
        }

        let v2 = if pad2 {
            0u32
        } else {
            b64_val(c2).ok_or_else(|| HostDiagError {
                code: invalid_code,
                message: "diag data base64 invalid char".to_string(),
            })? as u32
        };
        let v3 = if pad3 {
            0u32
        } else {
            b64_val(c3).ok_or_else(|| HostDiagError {
                code: invalid_code,
                message: "diag data base64 invalid char".to_string(),
            })? as u32
        };

        if pad2 && !pad3 {
            return Err(HostDiagError {
                code: invalid_code,
                message: "diag data base64 invalid padding".to_string(),
            });
        }

        let v = (v0 << 18) | (v1 << 12) | (v2 << 6) | v3;
        out.push(((v >> 16) & 0xff) as u8);
        if !pad2 {
            out.push(((v >> 8) & 0xff) as u8);
        }
        if !pad3 {
            out.push((v & 0xff) as u8);
        }
    }

    if out.len() > MAX_DATA_DECODED_BYTES {
        return Err(HostDiagError {
            code: invalid_code,
            message: "diag data decoded too long".to_string(),
        });
    }

    Ok(out)
}

fn parse_data_obj(decoded: &[u8], invalid_code: &'static str) -> Result<Value, HostDiagError> {
    let v: Value = serde_json::from_slice(decoded).map_err(|_| HostDiagError {
        code: invalid_code,
        message: "diag data is not valid JSON".to_string(),
    })?;
    if !v.is_object() {
        return Err(HostDiagError {
            code: invalid_code,
            message: "diag data must be a JSON object".to_string(),
        });
    }
    Ok(v)
}

pub fn extract_guest_diag_from_http_headers(
    headers: &[(String, String)],
) -> Result<Option<GuestDiag>, HostDiagError> {
    let mut code_val: Option<&str> = None;
    let mut data_val: Option<&str> = None;

    for (k, v) in headers {
        if k.eq_ignore_ascii_case(HTTP_HEADER_DIAG_CODE) {
            if code_val.is_some() {
                return Err(HostDiagError {
                    code: "X07WASM_HOST_DIAG_HTTP_DIAG_CODE_DUPLICATE",
                    message: "duplicate x-x07-diag-code header".to_string(),
                });
            }
            code_val = Some(v.as_str());
        }
        if k.eq_ignore_ascii_case(HTTP_HEADER_DIAG_DATA_B64) {
            if data_val.is_some() {
                return Err(HostDiagError {
                    code: "X07WASM_HOST_DIAG_HTTP_DIAG_DATA_DUPLICATE",
                    message: "duplicate x-x07-diag-data-b64 header".to_string(),
                });
            }
            data_val = Some(v.as_str());
        }
    }

    if data_val.is_some() && code_val.is_none() {
        return Err(HostDiagError {
            code: "X07WASM_HOST_DIAG_HTTP_DIAG_DATA_ORPHAN",
            message: "x-x07-diag-data-b64 present without x-x07-diag-code".to_string(),
        });
    }

    let Some(code_val) = code_val else {
        return Ok(None);
    };

    let code_trimmed = trim_ows(code_val.as_bytes());
    let code = parse_diag_code(code_trimmed, "X07WASM_HOST_DIAG_HTTP_DIAG_CODE_INVALID")?;

    let data_obj = if let Some(data_val) = data_val {
        let trimmed = trim_ows(data_val.as_bytes());
        let decoded = strict_base64_decode(trimmed, "X07WASM_HOST_DIAG_HTTP_DIAG_DATA_INVALID")?;
        Some(parse_data_obj(
            &decoded,
            "X07WASM_HOST_DIAG_HTTP_DIAG_DATA_INVALID",
        )?)
    } else {
        None
    };

    Ok(Some(GuestDiag { code, data_obj }))
}

fn count_line_start_matches(scan: &[u8], prefix: &[u8]) -> (usize, Option<usize>) {
    let mut count = 0usize;
    let mut pos: Option<usize> = None;
    for i in 0..scan.len() {
        let is_line_start = i == 0 || scan[i - 1] == b'\n';
        if !is_line_start {
            continue;
        }
        if scan[i..].starts_with(prefix) {
            count += 1;
            if pos.is_none() {
                pos = Some(i);
            }
        }
    }
    (count, pos)
}

fn parse_sentinel_line_value<'a>(scan: &'a [u8], pos: usize, prefix: &[u8]) -> &'a [u8] {
    let start = pos + prefix.len();
    let mut end = start;
    while end < scan.len() && scan[end] != b'\n' {
        end += 1;
    }
    if end > start && scan[end - 1] == b'\r' {
        end -= 1;
    }
    trim_ows(&scan[start..end])
}

pub fn extract_guest_diag_from_stderr(stderr: &[u8]) -> Result<Option<GuestDiag>, HostDiagError> {
    let scan_len = std::cmp::min(stderr.len(), MAX_SCAN_BYTES);
    let scan = &stderr[..scan_len];

    let (code_count, code_pos) = count_line_start_matches(scan, CLI_SENTINEL_DIAG_CODE);
    if code_count > 1 {
        return Err(HostDiagError {
            code: "X07WASM_HOST_DIAG_CLI_DIAG_CODE_DUPLICATE",
            message: "duplicate x07-diag-code sentinel line".to_string(),
        });
    }

    let (data_count, data_pos) = count_line_start_matches(scan, CLI_SENTINEL_DIAG_DATA_B64);
    if data_count > 1 {
        return Err(HostDiagError {
            code: "X07WASM_HOST_DIAG_CLI_DIAG_DATA_DUPLICATE",
            message: "duplicate x07-diag-data-b64 sentinel line".to_string(),
        });
    }

    if data_pos.is_some() && code_pos.is_none() {
        return Err(HostDiagError {
            code: "X07WASM_HOST_DIAG_CLI_DIAG_DATA_ORPHAN",
            message: "x07-diag-data-b64 present without x07-diag-code".to_string(),
        });
    }

    let Some(code_pos) = code_pos else {
        return Ok(None);
    };

    let code_bytes = parse_sentinel_line_value(scan, code_pos, CLI_SENTINEL_DIAG_CODE);
    let code = parse_diag_code(code_bytes, "X07WASM_HOST_DIAG_CLI_DIAG_CODE_INVALID")?;

    let data_obj = if let Some(data_pos) = data_pos {
        let data_bytes = parse_sentinel_line_value(scan, data_pos, CLI_SENTINEL_DIAG_DATA_B64);
        let decoded = strict_base64_decode(data_bytes, "X07WASM_HOST_DIAG_CLI_DIAG_DATA_INVALID")?;
        Some(parse_data_obj(
            &decoded,
            "X07WASM_HOST_DIAG_CLI_DIAG_DATA_INVALID",
        )?)
    } else {
        None
    };

    Ok(Some(GuestDiag { code, data_obj }))
}
