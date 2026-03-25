use std::time::Instant;

use serde_json::json;

fn main() {
    let iters: u64 = std::env::var("X07_WASM_MICROBENCH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200_000);

    let utf8 = br#"{"hello":"world","n":123,"ok":true}"#;
    let binary = vec![0xffu8; 1024];

    let _ = x07_wasm::stream_payload::bytes_to_stream_payload(utf8);
    let _ = x07_wasm::stream_payload::bytes_to_stream_payload_compact(utf8);

    let full_utf8 = bench(iters, || {
        let _ = x07_wasm::stream_payload::bytes_to_stream_payload(utf8);
    });
    let compact_utf8 = bench(iters, || {
        let _ = x07_wasm::stream_payload::bytes_to_stream_payload_compact(utf8);
    });

    let full_binary = bench(iters, || {
        let _ = x07_wasm::stream_payload::bytes_to_stream_payload(&binary);
    });
    let compact_binary = bench(iters, || {
        let _ = x07_wasm::stream_payload::bytes_to_stream_payload_compact(&binary);
    });

    let doc = json!({
        "iters": iters,
        "cases": {
            "utf8": {
                "bytes_len": utf8.len(),
                "full_total_ns": full_utf8,
                "compact_total_ns": compact_utf8,
                "speedup": speedup(full_utf8, compact_utf8),
            },
            "binary_1024": {
                "bytes_len": binary.len(),
                "full_total_ns": full_binary,
                "compact_total_ns": compact_binary,
                "speedup": speedup(full_binary, compact_binary),
            }
        }
    });

    println!("{}", doc);
}

fn bench<F>(iters: u64, mut f: F) -> u128
where
    F: FnMut(),
{
    let started = Instant::now();
    for _ in 0..iters {
        f();
    }
    started.elapsed().as_nanos()
}

fn speedup(full_ns: u128, compact_ns: u128) -> f64 {
    if compact_ns == 0 {
        return 0.0;
    }
    (full_ns as f64) / (compact_ns as f64)
}
