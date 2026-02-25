#![no_std]

extern crate alloc;
extern crate rlibc;

use alloc::string::String;
use alloc::vec::Vec;
use base64::Engine as _;
use core::panic::PanicInfo;

wit_bindgen::generate!({
    world: "x07:http-adapter/proxy-with-solve@0.1.0",
    path: [
        "../../wit/deps/wasi/io/0.2.8",
        "../../wit/deps/wasi/random/0.2.8",
        "../../wit/deps/wasi/clocks/0.2.8",
        "../../wit/deps/wasi/sockets/0.2.8",
        "../../wit/deps/wasi/filesystem/0.2.8",
        "../../wit/deps/wasi/cli/0.2.8",
        "../../wit/deps/wasi/http/0.2.8",
        "../../wit/x07/solve/0.1.0",
        "../../wit/x07/http_adapter/0.1.0",
    ],
    generate_all,
});

#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[no_mangle]
pub unsafe extern "C" fn cabi_realloc(
    old_ptr: *mut u8,
    old_len: usize,
    align: usize,
    new_len: usize,
) -> *mut u8 {
    use alloc::alloc::{Layout, alloc, handle_alloc_error, realloc};

    let layout;
    let ptr = if old_len == 0 {
        if new_len == 0 {
            return align as *mut u8;
        }
        layout = Layout::from_size_align_unchecked(new_len, align);
        alloc(layout)
    } else {
        debug_assert_ne!(new_len, 0, "non-zero old_len requires non-zero new_len");
        layout = Layout::from_size_align_unchecked(old_len, align);
        realloc(old_ptr, layout, new_len)
    };

    if ptr.is_null() {
        handle_alloc_error(layout);
    }
    ptr
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    #[cfg(target_arch = "wasm32")]
    core::arch::wasm32::unreachable();

    #[allow(unreachable_code)]
    loop {}
}

struct HttpResponseEnvelope {
    v: u32,
    kind: String,
    status: u16,
    headers: Vec<(String, String)>,
    body_b64: String,
}

struct HttpAdapter;

impl exports::wasi::http::incoming_handler::Guest for HttpAdapter {
    fn handle(request: wasi::http::types::IncomingRequest, response_out: wasi::http::types::ResponseOutparam) {
        let method = http_method_string(request.method());
        let (path, query) = http_path_and_query(request.path_with_query());

        let headers_resource = request.headers();
        let headers = headers_resource
            .entries()
            .into_iter()
            .map(|(k, v)| (k, String::from_utf8_lossy(&v).into_owned()))
            .collect::<Vec<_>>();
        drop(headers_resource);

        let body_bytes = match request.consume() {
            Ok(body) => read_incoming_body(body),
            Err(()) => Vec::new(),
        };

        let body_b64 = base64::engine::general_purpose::STANDARD.encode(body_bytes);
        let env_bytes = request_envelope_bytes(&method, &path, &query, &headers, &body_b64);

        let resp_bytes = x07::solve::handler::solve(&env_bytes);
        let resp: HttpResponseEnvelope = parse_response_envelope(&resp_bytes);
        if resp.v != 1 || resp.kind != "x07.http.response" {
            panic_invalid_response_envelope();
        }

        let body = base64::engine::general_purpose::STANDARD
            .decode(resp.body_b64.as_bytes())
            .unwrap_or_else(|_| panic_invalid_response_envelope());

        let header_entries = resp
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.as_bytes().to_vec()))
            .collect::<Vec<_>>();
        let fields = wasi::http::types::Fields::from_list(&header_entries)
            .expect("response headers invalid for wasi:http");

        let outgoing = wasi::http::types::OutgoingResponse::new(fields);
        outgoing
            .set_status_code(resp.status)
            .expect("invalid status code");

        let out_body = outgoing.body().expect("outgoing-response.body failed");
        if !body.is_empty() {
            let out_stream = out_body.write().expect("outgoing-body.write failed");
            for chunk in body.chunks(4096) {
                out_stream
                    .blocking_write_and_flush(chunk)
                    .expect("write response body");
            }
            drop(out_stream);
        }
        wasi::http::types::OutgoingBody::finish(out_body, None).expect("finish response body");

        wasi::http::types::ResponseOutparam::set(response_out, Ok(outgoing));
    }
}

export!(HttpAdapter);

fn panic_invalid_response_envelope() -> ! {
    #[cfg(target_arch = "wasm32")]
    core::arch::wasm32::unreachable();

    #[allow(unreachable_code)]
    loop {}
}

fn http_method_string(method: wasi::http::types::Method) -> String {
    match method {
        wasi::http::types::Method::Get => String::from("GET"),
        wasi::http::types::Method::Head => String::from("HEAD"),
        wasi::http::types::Method::Post => String::from("POST"),
        wasi::http::types::Method::Put => String::from("PUT"),
        wasi::http::types::Method::Delete => String::from("DELETE"),
        wasi::http::types::Method::Connect => String::from("CONNECT"),
        wasi::http::types::Method::Options => String::from("OPTIONS"),
        wasi::http::types::Method::Trace => String::from("TRACE"),
        wasi::http::types::Method::Patch => String::from("PATCH"),
        wasi::http::types::Method::Other(s) => s,
    }
}

fn http_path_and_query(path_with_query: Option<String>) -> (String, String) {
    let Some(s) = path_with_query else {
        return (String::from("/"), String::new());
    };
    match s.split_once('?') {
        Some((p, q)) => (String::from(p), String::from(q)),
        None => (s, String::new()),
    }
}

fn read_incoming_body(body: wasi::http::types::IncomingBody) -> Vec<u8> {
    let stream = match body.stream() {
        Ok(s) => s,
        Err(()) => {
            let _ = wasi::http::types::IncomingBody::finish(body);
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    loop {
        match stream.blocking_read(4096) {
            Ok(chunk) => {
                if chunk.is_empty() {
                    break;
                }
                out.extend_from_slice(&chunk);
            }
            Err(wasi::io::streams::StreamError::Closed) => break,
            Err(wasi::io::streams::StreamError::LastOperationFailed(e)) => {
                panic!("read body failed: {}", e.to_debug_string());
            }
        }
    }

    drop(stream);
    let _trailers = wasi::http::types::IncomingBody::finish(body);
    out
}

fn request_envelope_bytes(
    method: &str,
    path: &str,
    query: &str,
    headers: &[(String, String)],
    body_b64: &str,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(br#"{"v":1,"kind":"x07.http.request","method":"#);
    push_json_string(&mut out, method);
    out.extend_from_slice(br#","path":"#);
    push_json_string(&mut out, path);
    out.extend_from_slice(br#","query":"#);
    push_json_string(&mut out, query);
    out.extend_from_slice(br#","headers":["#);
    for (i, (k, v)) in headers.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.push(b'[');
        push_json_string(&mut out, k);
        out.push(b',');
        push_json_string(&mut out, v);
        out.push(b']');
    }
    out.extend_from_slice(br#"],"body_b64":"#);
    push_json_string(&mut out, body_b64);
    out.extend_from_slice(br#"}"#);
    out
}

fn push_json_string(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    for &b in s.as_bytes() {
        match b {
            b'"' => out.extend_from_slice(br#"\""#),
            b'\\' => out.extend_from_slice(br#"\\"#),
            0x00..=0x1f => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.extend_from_slice(br#"\u00"#);
                out.push(HEX[(b >> 4) as usize]);
                out.push(HEX[(b & 0xf) as usize]);
            }
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

fn parse_response_envelope(bytes: &[u8]) -> HttpResponseEnvelope {
    let mut cur = Cursor::new(bytes);
    cur.skip_ws();
    cur.expect_byte(b'{');

    let mut v: Option<u32> = None;
    let mut kind: Option<String> = None;
    let mut status: Option<u16> = None;
    let mut headers: Option<Vec<(String, String)>> = None;
    let mut body_b64: Option<String> = None;

    cur.skip_ws();
    if cur.peek_byte() == Some(b'}') {
        panic_invalid_response_envelope();
    }

    loop {
        cur.skip_ws();
        let key = cur.parse_string();
        cur.skip_ws();
        cur.expect_byte(b':');
        cur.skip_ws();

        match key.as_str() {
            "v" => v = Some(cur.parse_u32()),
            "kind" => kind = Some(cur.parse_string()),
            "status" => status = Some(cur.parse_u16()),
            "headers" => headers = Some(cur.parse_headers()),
            "body_b64" => body_b64 = Some(cur.parse_string()),
            _ => cur.skip_value(),
        }

        cur.skip_ws();
        match cur.next_byte() {
            Some(b',') => continue,
            Some(b'}') => break,
            _ => panic_invalid_response_envelope(),
        }
    }

    let Some(v) = v else {
        panic_invalid_response_envelope();
    };
    let Some(kind) = kind else {
        panic_invalid_response_envelope();
    };
    let Some(status) = status else {
        panic_invalid_response_envelope();
    };
    let Some(headers) = headers else {
        panic_invalid_response_envelope();
    };
    let Some(body_b64) = body_b64 else {
        panic_invalid_response_envelope();
    };

    HttpResponseEnvelope {
        v,
        kind,
        status,
        headers,
        body_b64,
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, i: 0 }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.i).copied()
    }

    fn next_byte(&mut self) -> Option<u8> {
        let b = self.peek_byte()?;
        self.i += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek_byte() {
            match b {
                b' ' | b'\n' | b'\r' | b'\t' => {
                    self.i += 1;
                }
                _ => break,
            }
        }
    }

    fn expect_byte(&mut self, wanted: u8) {
        match self.next_byte() {
            Some(b) if b == wanted => {}
            _ => panic_invalid_response_envelope(),
        }
    }

    fn parse_u32(&mut self) -> u32 {
        let mut n: u64 = 0;
        let mut any = false;
        while let Some(b) = self.peek_byte() {
            if !(b'0'..=b'9').contains(&b) {
                break;
            }
            any = true;
            n = n * 10 + u64::from(b - b'0');
            if n > u64::from(u32::MAX) {
                panic_invalid_response_envelope();
            }
            self.i += 1;
        }
        if !any {
            panic_invalid_response_envelope();
        }
        n as u32
    }

    fn parse_u16(&mut self) -> u16 {
        let n = self.parse_u32();
        u16::try_from(n).unwrap_or_else(|_| panic_invalid_response_envelope())
    }

    fn parse_string(&mut self) -> String {
        self.expect_byte(b'"');
        let mut out: Vec<u8> = Vec::new();
        while let Some(b) = self.next_byte() {
            match b {
                b'"' => {
                    return String::from_utf8(out)
                        .unwrap_or_else(|_| panic_invalid_response_envelope());
                }
                b'\\' => {
                    let Some(esc) = self.next_byte() else {
                        panic_invalid_response_envelope();
                    };
                    match esc {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let c = self.parse_u_escape();
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => panic_invalid_response_envelope(),
                    }
                }
                0x00..=0x1f => panic_invalid_response_envelope(),
                _ => out.push(b),
            }
        }
        panic_invalid_response_envelope();
    }

    fn parse_u_escape(&mut self) -> char {
        let a = self.parse_hex4();
        if (0xD800..=0xDBFF).contains(&a) {
            let saved = self.i;
            if self.next_byte() != Some(b'\\') || self.next_byte() != Some(b'u') {
                self.i = saved;
                panic_invalid_response_envelope();
            }
            let b = self.parse_hex4();
            if !(0xDC00..=0xDFFF).contains(&b) {
                panic_invalid_response_envelope();
            }
            let hi = a - 0xD800;
            let lo = b - 0xDC00;
            let cp = 0x10000 + ((hi as u32) << 10) + (lo as u32);
            return core::char::from_u32(cp).unwrap_or_else(|| panic_invalid_response_envelope());
        }
        core::char::from_u32(a as u32).unwrap_or_else(|| panic_invalid_response_envelope())
    }

    fn parse_hex4(&mut self) -> u16 {
        let mut v: u16 = 0;
        for _ in 0..4 {
            let Some(b) = self.next_byte() else {
                panic_invalid_response_envelope();
            };
            let d = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => 10 + (b - b'a'),
                b'A'..=b'F' => 10 + (b - b'A'),
                _ => panic_invalid_response_envelope(),
            };
            v = (v << 4) | u16::from(d);
        }
        v
    }

    fn parse_headers(&mut self) -> Vec<(String, String)> {
        self.expect_byte(b'[');
        let mut out: Vec<(String, String)> = Vec::new();
        self.skip_ws();
        if self.peek_byte() == Some(b']') {
            self.i += 1;
            return out;
        }
        loop {
            self.skip_ws();
            self.expect_byte(b'[');
            self.skip_ws();
            let k = self.parse_string();
            self.skip_ws();
            self.expect_byte(b',');
            self.skip_ws();
            let v = self.parse_string();
            self.skip_ws();
            self.expect_byte(b']');
            out.push((k, v));

            self.skip_ws();
            match self.next_byte() {
                Some(b',') => continue,
                Some(b']') => break,
                _ => panic_invalid_response_envelope(),
            }
        }
        out
    }

    fn skip_value(&mut self) {
        self.skip_ws();
        match self.peek_byte() {
            Some(b'{') => self.skip_object(),
            Some(b'[') => self.skip_array(),
            Some(b'"') => {
                let _ = self.parse_string();
            }
            Some(b'-') | Some(b'0'..=b'9') => {
                self.skip_number();
            }
            Some(b't') => self.expect_bytes(b"true"),
            Some(b'f') => self.expect_bytes(b"false"),
            Some(b'n') => self.expect_bytes(b"null"),
            _ => panic_invalid_response_envelope(),
        }
    }

    fn skip_number(&mut self) {
        if self.peek_byte() == Some(b'-') {
            self.i += 1;
        }
        let mut any = false;
        while let Some(b) = self.peek_byte() {
            if !(b'0'..=b'9').contains(&b) {
                break;
            }
            any = true;
            self.i += 1;
        }
        if !any {
            panic_invalid_response_envelope();
        }
        if self.peek_byte() == Some(b'.') {
            self.i += 1;
            let mut any_frac = false;
            while let Some(b) = self.peek_byte() {
                if !(b'0'..=b'9').contains(&b) {
                    break;
                }
                any_frac = true;
                self.i += 1;
            }
            if !any_frac {
                panic_invalid_response_envelope();
            }
        }
        if matches!(self.peek_byte(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek_byte(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            let mut any_exp = false;
            while let Some(b) = self.peek_byte() {
                if !(b'0'..=b'9').contains(&b) {
                    break;
                }
                any_exp = true;
                self.i += 1;
            }
            if !any_exp {
                panic_invalid_response_envelope();
            }
        }
    }

    fn skip_object(&mut self) {
        self.expect_byte(b'{');
        self.skip_ws();
        if self.peek_byte() == Some(b'}') {
            self.i += 1;
            return;
        }
        loop {
            self.skip_ws();
            let _ = self.parse_string();
            self.skip_ws();
            self.expect_byte(b':');
            self.skip_ws();
            self.skip_value();
            self.skip_ws();
            match self.next_byte() {
                Some(b',') => continue,
                Some(b'}') => break,
                _ => panic_invalid_response_envelope(),
            }
        }
    }

    fn skip_array(&mut self) {
        self.expect_byte(b'[');
        self.skip_ws();
        if self.peek_byte() == Some(b']') {
            self.i += 1;
            return;
        }
        loop {
            self.skip_ws();
            self.skip_value();
            self.skip_ws();
            match self.next_byte() {
                Some(b',') => continue,
                Some(b']') => break,
                _ => panic_invalid_response_envelope(),
            }
        }
    }

    fn expect_bytes(&mut self, s: &[u8]) {
        for &b in s {
            if self.next_byte() != Some(b) {
                panic_invalid_response_envelope();
            }
        }
    }
}
