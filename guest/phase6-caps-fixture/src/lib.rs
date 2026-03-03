wit_bindgen::generate!({
    world: "x07:phase6-caps-fixture/proxy@0.1.0",
    path: [
        "../../wit/deps/wasi/io/0.2.8",
        "../../wit/deps/wasi/random/0.2.8",
        "../../wit/deps/wasi/clocks/0.2.8",
        "../../wit/deps/wasi/sockets/0.2.8",
        "../../wit/deps/wasi/filesystem/0.2.8",
        "../../wit/deps/wasi/cli/0.2.8",
        "../../wit/deps/wasi/http/0.2.8",
        "../../wit/x07/phase6_caps_fixture/0.1.0",
    ],
    generate_all,
});

const DIAG_HEADER_CODE: &str = "x-x07-diag-code";

struct Phase6CapsFixture;

impl exports::wasi::http::incoming_handler::Guest for Phase6CapsFixture {
    fn handle(
        request: wasi::http::types::IncomingRequest,
        response_out: wasi::http::types::ResponseOutparam,
    ) {
        let (path, _query) = http_path_and_query(request.path_with_query());
        drop(request);

        match path.as_str() {
            "/fs" => match std::fs::read_to_string("spec/fixtures/phase6_caps_fs/hello.txt") {
                Ok(s) => respond_ok(response_out, s.trim().as_bytes()),
                Err(_) => respond_error(response_out, "X07WASM_CAPS_FS_DENIED", b"fs denied"),
            },
            "/secret" => match std::env::var("X07_SECRET_API_KEY") {
                Ok(_) => respond_ok(response_out, b"ok"),
                Err(_) => respond_error(
                    response_out,
                    "X07WASM_CAPS_SECRET_DENIED",
                    b"secret denied",
                ),
            },
            "/time_rand" => {
                let wall = wasi::clocks::wall_clock::now();
                let mono = wasi::clocks::monotonic_clock::now();
                let rand = wasi::random::random::get_random_bytes(16);
                let body = format!(
                    "wall={}.{:09} mono={} rand={}\n",
                    wall.seconds,
                    wall.nanoseconds,
                    mono,
                    hex_lower(&rand)
                );
                respond_ok(response_out, body.as_bytes());
            }
            "/net_ip_literal" => {
                let headers = wasi::http::types::Fields::new();
                let outgoing = wasi::http::types::OutgoingRequest::new(headers);
                outgoing
                    .set_scheme(Some(&wasi::http::types::Scheme::Http))
                    .expect("set_scheme failed");
                outgoing
                    .set_authority(Some("127.0.0.1:80"))
                    .expect("set_authority failed");
                outgoing
                    .set_path_with_query(Some("/"))
                    .expect("set_path_with_query failed");

                match wasi::http::outgoing_handler::handle(outgoing, None) {
                    Ok(_future) => respond_ok(response_out, b"unexpected ok"),
                    Err(_) => respond_error(
                        response_out,
                        "X07WASM_CAPS_NET_DENIED",
                        b"network denied",
                    ),
                }
            }
            "/net_default_port_http" => {
                let headers = wasi::http::types::Fields::new();
                let outgoing = wasi::http::types::OutgoingRequest::new(headers);
                outgoing
                    .set_scheme(Some(&wasi::http::types::Scheme::Http))
                    .expect("set_scheme failed");
                outgoing
                    .set_authority(Some("localhost"))
                    .expect("set_authority failed");
                outgoing
                    .set_path_with_query(Some("/"))
                    .expect("set_path_with_query failed");

                match wasi::http::outgoing_handler::handle(outgoing, None) {
                    Ok(_future) => respond_ok(response_out, b"ok"),
                    Err(_) => respond_error(
                        response_out,
                        "X07WASM_CAPS_NET_DENIED",
                        b"network denied",
                    ),
                }
            }
            _ => respond_ok(response_out, b"ok"),
        }
    }
}

export!(Phase6CapsFixture);

fn respond_ok(response_out: wasi::http::types::ResponseOutparam, body: &[u8]) {
    respond_with_headers(response_out, 200, &[], body);
}

fn respond_error(
    response_out: wasi::http::types::ResponseOutparam,
    code: &str,
    body: &[u8],
) {
    respond_with_headers(
        response_out,
        500,
        &[(DIAG_HEADER_CODE, code.as_bytes())],
        body,
    );
}

fn respond_with_headers(
    response_out: wasi::http::types::ResponseOutparam,
    status: u16,
    headers: &[(&str, &[u8])],
    body: &[u8],
) {
    let header_entries = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_vec()))
        .collect::<Vec<_>>();
    let fields =
        wasi::http::types::Fields::from_list(&header_entries).expect("response headers invalid");

    let outgoing = wasi::http::types::OutgoingResponse::new(fields);
    outgoing
        .set_status_code(status)
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

fn http_path_and_query(path_with_query: Option<String>) -> (String, String) {
    let Some(s) = path_with_query else {
        return (String::from("/"), String::new());
    };
    match s.split_once('?') {
        Some((p, q)) => (String::from(p), String::from(q)),
        None => (s, String::new()),
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
