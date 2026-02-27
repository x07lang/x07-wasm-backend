use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use wasmtime_wasi::{
    Deterministic, DirPerms, FilePerms, HostMonotonicClock, HostWallClock, WasiCtx,
};
use wasmtime_wasi::{SocketAddrUse, WasiCtxBuilder};

use crate::caps::doc::{CapabilitiesDoc, CapabilityMode, FsPreopenMode, NetworkMode, NetworkProto};
use crate::diag::{Diagnostic, Severity, Stage};

pub fn build_wasi_ctx_from_caps(
    caps: &CapabilitiesDoc,
    base_dir: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<WasiCtx>> {
    let mut wasi = WasiCtxBuilder::new();

    // fs.preopens
    for p in &caps.fs.preopens {
        let host_dir = base_dir.join(&p.path);
        let guest_path = &p.path;
        let (dir_perms, file_perms) = match p.mode {
            FsPreopenMode::Ro => (DirPerms::READ, FilePerms::READ),
            FsPreopenMode::Rw => (DirPerms::all(), FilePerms::all()),
        };
        if let Err(err) = wasi.preopened_dir(&host_dir, guest_path, dir_perms, file_perms) {
            diagnostics.push(Diagnostic::new(
                "X07WASM_CAPS_FS_DENIED",
                Severity::Error,
                Stage::Parse,
                format!(
                    "failed to preopen dir {:?} (guest path {:?}): {err:#}",
                    host_dir.display(),
                    guest_path
                ),
            ));
            return Ok(None);
        }
    }

    // env.allow
    for key in &caps.env.allow {
        if let Ok(v) = std::env::var(key) {
            wasi.env(key, v);
        }
    }

    // secrets.allow (provider v1: env + file)
    let secrets_dir = std::env::var("X07_SECRETS_DIR").ok();
    let secrets_dir = secrets_dir.as_deref().map(Path::new);
    for secret_id in &caps.secrets.allow {
        let env_key = secret_env_key(secret_id);
        if let Some(v) = resolve_secret_value(secret_id, &env_key, secrets_dir) {
            wasi.env(env_key, v);
        }
    }

    // network allowlist for raw sockets: only literal IPs are supported in Phase 6.
    if caps.network.mode == NetworkMode::Allowlist {
        let mut allowed: std::collections::BTreeSet<SocketAddr> = std::collections::BTreeSet::new();
        for ep in &caps.network.allowlist {
            if !matches!(ep.proto, NetworkProto::Tcp | NetworkProto::Udp) {
                continue;
            }
            let Ok(ip) = ep.host.parse::<IpAddr>() else {
                continue;
            };
            allowed.insert(SocketAddr::new(ip, ep.port));
        }
        if !allowed.is_empty() {
            let allowed = Arc::new(allowed);
            wasi.socket_addr_check(move |addr, _use_: SocketAddrUse| {
                let allowed = allowed.clone();
                Box::pin(async move { allowed.contains(&addr) })
                    as Pin<Box<dyn Future<Output = bool> + Send + Sync>>
            });
        }
    }

    // clocks
    if caps.clocks.mode == CapabilityMode::Deny {
        wasi.wall_clock(ZeroWallClock)
            .monotonic_clock(ZeroMonotonicClock);
    }

    // random
    if caps.random.mode == CapabilityMode::Deny {
        wasi.secure_random(Deterministic::new(vec![0]))
            .insecure_random(Deterministic::new(vec![0]));
    }

    Ok(Some(wasi.build()))
}

fn resolve_secret_value(
    secret_id: &str,
    env_key: &str,
    secrets_dir: Option<&Path>,
) -> Option<String> {
    if let Some(dir) = secrets_dir {
        let path = dir.join(secret_id);
        if let Ok(bytes) = std::fs::read(&path) {
            let s = String::from_utf8_lossy(&bytes);
            return Some(s.trim_end_matches(['\n', '\r']).to_string());
        }
    }

    std::env::var(env_key).ok()
}

fn secret_env_key(secret_id: &str) -> String {
    let mut out = String::from("X07_SECRET_");
    for ch in secret_id.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_uppercase());
        } else {
            out.push('_');
        }
    }
    out
}

struct ZeroWallClock;

impl HostWallClock for ZeroWallClock {
    fn resolution(&self) -> std::time::Duration {
        std::time::Duration::from_nanos(1)
    }

    fn now(&self) -> std::time::Duration {
        std::time::Duration::from_secs(0)
    }
}

struct ZeroMonotonicClock;

impl HostMonotonicClock for ZeroMonotonicClock {
    fn resolution(&self) -> u64 {
        1
    }

    fn now(&self) -> u64 {
        0
    }
}
