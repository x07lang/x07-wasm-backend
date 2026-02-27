use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use wasmtime_wasi::{
    Deterministic, DirPerms, FilePerms, HostMonotonicClock, HostWallClock, WasiCtx,
};
use wasmtime_wasi::{SocketAddrUse, WasiCtxBuilder};

use crate::caps::doc::{CapabilitiesDoc, CapabilityMode, FsPreopenMode, NetworkMode, NetworkProto};
use crate::caps::evidence::{CapsEvidenceCtx, CapsEvidenceMode};
use crate::diag::{Diagnostic, Severity, Stage};

pub fn build_wasi_ctx_from_caps(
    caps: &CapabilitiesDoc,
    base_dir: &Path,
    evidence: Option<&CapsEvidenceCtx>,
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
    for secret_id in &caps.secrets.allow {
        let env_key = secret_env_key(secret_id);
        match resolve_secret_value(base_dir, secret_id, &env_key) {
            Ok(Some((v, source))) => {
                wasi.env(env_key, v);
                if let Some(ev) = evidence {
                    if ev.mode() == CapsEvidenceMode::Record {
                        ev.record_secret_provided(secret_id, source);
                    }
                }
            }
            Ok(None) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_CAPS_PROFILE_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("missing secret: {secret_id:?}"),
                ));
                return Ok(None);
            }
            Err(err) => {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_CAPS_PROFILE_READ_FAILED",
                    Severity::Error,
                    Stage::Parse,
                    format!("secret provider failed for {secret_id:?}: {err:#}"),
                ));
                return Ok(None);
            }
        }
    }

    // network allowlist for raw sockets: only literal IPs are supported in Phase 6.
    //
    // Deny-by-default: if the allowlist is empty (or if caps say deny), all socket addresses are
    // rejected.
    let mut allowed: std::collections::BTreeSet<SocketAddr> = std::collections::BTreeSet::new();
    if caps.network.mode == NetworkMode::Allowlist {
        for ep in &caps.network.allowlist {
            if !matches!(ep.proto, NetworkProto::Tcp | NetworkProto::Udp) {
                continue;
            }
            let Ok(ip) = ep.host.parse::<IpAddr>() else {
                continue;
            };
            allowed.insert(SocketAddr::new(ip, ep.port));
        }
    }
    let allowed = Arc::new(allowed);
    wasi.socket_addr_check(move |addr, _use_: SocketAddrUse| {
        let allowed = allowed.clone();
        Box::pin(async move { allowed.contains(&addr) })
            as Pin<Box<dyn Future<Output = bool> + Send + Sync>>
    });

    // clocks
    match caps.clocks.mode {
        CapabilityMode::Deny => {
            wasi.wall_clock(ZeroWallClock)
                .monotonic_clock(ZeroMonotonicClock);
        }
        CapabilityMode::Allow => {}
        CapabilityMode::Record => {
            let Some(ev) = evidence else {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_POLICY_OBLIGATION_UNSATISFIED",
                    Severity::Error,
                    Stage::Parse,
                    "clocks.mode=record requires --evidence-in or --evidence-out".to_string(),
                ));
                return Ok(None);
            };
            wasi.wall_clock(EvidenceWallClock {
                evidence: ev.clone(),
            })
            .monotonic_clock(EvidenceMonotonicClock {
                evidence: ev.clone(),
                base: Instant::now(),
            });
        }
    }

    // random
    match caps.random.mode {
        CapabilityMode::Deny => {
            wasi.secure_random(Deterministic::new(vec![0]))
                .insecure_random(Deterministic::new(vec![0]))
                .insecure_random_seed(0);
        }
        CapabilityMode::Allow => {}
        CapabilityMode::Record => {
            let Some(ev) = evidence else {
                diagnostics.push(Diagnostic::new(
                    "X07WASM_POLICY_OBLIGATION_UNSATISFIED",
                    Severity::Error,
                    Stage::Parse,
                    "random.mode=record requires --evidence-in or --evidence-out".to_string(),
                ));
                return Ok(None);
            };

            if ev.mode() == CapsEvidenceMode::Record {
                let seed = random_u128();
                ev.set_insecure_seed_u128(seed);
                wasi.insecure_random_seed(seed);
            } else {
                wasi.insecure_random_seed(ev.insecure_seed_u128());
            }

            wasi.secure_random(EvidenceRng::new(ev.clone(), EvidenceRngKind::Secure))
                .insecure_random(EvidenceRng::new(ev.clone(), EvidenceRngKind::Insecure));
        }
    }

    Ok(Some(wasi.build()))
}

fn resolve_secret_value(
    base_dir: &Path,
    secret_id: &str,
    env_key: &str,
) -> Result<Option<(String, &'static str)>> {
    let dir = resolve_secrets_dir(base_dir);
    let path = dir.join(secret_id);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let s = String::from_utf8_lossy(&bytes);
            return Ok(Some((s.trim_end_matches(['\n', '\r']).to_string(), "file")));
        }
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(anyhow::Error::new(err));
            }
        }
    }

    match std::env::var(env_key) {
        Ok(v) => Ok(Some((v, "env"))),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(anyhow::anyhow!("secret env var is not valid unicode"))
        }
    }
}

fn resolve_secrets_dir(base_dir: &Path) -> std::path::PathBuf {
    if let Ok(s) = std::env::var("X07_SECRETS_DIR") {
        let p = std::path::PathBuf::from(s);
        if p.is_absolute() {
            return p;
        }
        return base_dir.join(p);
    }
    base_dir.join(".x07").join("secrets")
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

struct EvidenceWallClock {
    evidence: CapsEvidenceCtx,
}

impl HostWallClock for EvidenceWallClock {
    fn resolution(&self) -> Duration {
        Duration::from_nanos(1)
    }

    fn now(&self) -> Duration {
        match self.evidence.mode() {
            CapsEvidenceMode::Record => {
                let d = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO);
                let ns = d.as_nanos().min(u64::MAX as u128) as u64;
                self.evidence.record_wall_clock_now_ns(ns);
                Duration::from_nanos(ns)
            }
            CapsEvidenceMode::Replay => self
                .evidence
                .replay_wall_clock_now_ns()
                .map(Duration::from_nanos)
                .unwrap_or(Duration::ZERO),
        }
    }
}

struct EvidenceMonotonicClock {
    evidence: CapsEvidenceCtx,
    base: Instant,
}

impl HostMonotonicClock for EvidenceMonotonicClock {
    fn resolution(&self) -> u64 {
        1
    }

    fn now(&self) -> u64 {
        match self.evidence.mode() {
            CapsEvidenceMode::Record => {
                let ns = self.base.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                self.evidence.record_monotonic_clock_now_ns(ns);
                ns
            }
            CapsEvidenceMode::Replay => self.evidence.replay_monotonic_clock_now_ns().unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum EvidenceRngKind {
    Secure,
    Insecure,
}

struct EvidenceRng {
    kind: EvidenceRngKind,
    evidence: CapsEvidenceCtx,
    rng: Option<cap_rand::rngs::StdRng>,
}

impl EvidenceRng {
    fn new(evidence: CapsEvidenceCtx, kind: EvidenceRngKind) -> Self {
        let rng = if evidence.mode() == CapsEvidenceMode::Record {
            use cap_rand::{Rng, SeedableRng};
            let mut seed_rng = cap_rand::thread_rng(cap_rand::ambient_authority());
            Some(cap_rand::rngs::StdRng::from_seed(seed_rng.r#gen()))
        } else {
            None
        };
        Self {
            kind,
            evidence,
            rng,
        }
    }

    fn record_bytes(&self, bytes: &[u8]) {
        match self.kind {
            EvidenceRngKind::Secure => self.evidence.record_secure_random_bytes(bytes),
            EvidenceRngKind::Insecure => self.evidence.record_insecure_random_bytes(bytes),
        }
    }

    fn replay_bytes_into(&self, out: &mut [u8]) {
        match self.kind {
            EvidenceRngKind::Secure => self.evidence.replay_secure_random_bytes_into(out),
            EvidenceRngKind::Insecure => self.evidence.replay_insecure_random_bytes_into(out),
        }
    }
}

impl cap_rand::RngCore for EvidenceRng {
    fn next_u32(&mut self) -> u32 {
        let mut buf = [0u8; 4];
        self.fill_bytes(&mut buf);
        u32::from_le_bytes(buf)
    }

    fn next_u64(&mut self) -> u64 {
        let mut buf = [0u8; 8];
        self.fill_bytes(&mut buf);
        u64::from_le_bytes(buf)
    }

    fn fill_bytes(&mut self, buf: &mut [u8]) {
        if self.evidence.mode() == CapsEvidenceMode::Replay {
            self.replay_bytes_into(buf);
            return;
        }

        if let Some(rng) = self.rng.as_mut() {
            rng.fill_bytes(buf);
            self.record_bytes(buf);
        } else {
            buf.fill(0);
        }
    }

    fn try_fill_bytes(&mut self, buf: &mut [u8]) -> std::result::Result<(), cap_rand::Error> {
        self.fill_bytes(buf);
        Ok(())
    }
}

fn random_u128() -> u128 {
    use cap_rand::Rng;
    let mut rng = cap_rand::thread_rng(cap_rand::ambient_authority());
    rng.r#gen::<u128>()
}
