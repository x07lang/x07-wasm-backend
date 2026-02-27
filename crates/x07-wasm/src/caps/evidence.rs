use std::sync::{Arc, Mutex};

use anyhow::Result;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

pub const CAPS_EVIDENCE_SCHEMA_VERSION: &str = "x07.wasm.caps.evidence@0.1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapsEvidenceMode {
    Record,
    Replay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsEvidenceDoc {
    pub schema_version: String,
    pub v: u64,
    #[serde(default)]
    pub clocks: CapsEvidenceClocks,
    #[serde(default)]
    pub random: CapsEvidenceRandom,
    #[serde(default)]
    pub secrets: CapsEvidenceSecrets,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapsEvidenceClocks {
    #[serde(default)]
    pub wall_clock_now_ns: Vec<u64>,
    #[serde(default)]
    pub monotonic_clock_now_ns: Vec<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapsEvidenceRandom {
    #[serde(default)]
    pub secure_random_bytes_b64: String,
    #[serde(default)]
    pub insecure_random_bytes_b64: String,
    #[serde(default)]
    pub insecure_seed_u128: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapsEvidenceSecrets {
    #[serde(default)]
    pub provided: Vec<CapsEvidenceSecretProvided>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsEvidenceSecretProvided {
    pub id: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct CapsEvidenceCtx {
    mode: CapsEvidenceMode,
    inner: Arc<Mutex<CapsEvidenceState>>,
}

#[derive(Debug)]
struct CapsEvidenceState {
    wall_clock_now_ns: Vec<u64>,
    monotonic_clock_now_ns: Vec<u64>,
    secure_random_bytes: Vec<u8>,
    insecure_random_bytes: Vec<u8>,
    insecure_seed_u128: u128,
    secrets_provided: Vec<CapsEvidenceSecretProvided>,

    wall_clock_i: usize,
    monotonic_clock_i: usize,
    secure_random_i: usize,
    insecure_random_i: usize,
    replay_errors: Vec<String>,
}

impl CapsEvidenceCtx {
    pub fn new_record() -> Self {
        Self {
            mode: CapsEvidenceMode::Record,
            inner: Arc::new(Mutex::new(CapsEvidenceState {
                wall_clock_now_ns: Vec::new(),
                monotonic_clock_now_ns: Vec::new(),
                secure_random_bytes: Vec::new(),
                insecure_random_bytes: Vec::new(),
                insecure_seed_u128: 0,
                secrets_provided: Vec::new(),
                wall_clock_i: 0,
                monotonic_clock_i: 0,
                secure_random_i: 0,
                insecure_random_i: 0,
                replay_errors: Vec::new(),
            })),
        }
    }

    pub fn new_replay(doc: &CapsEvidenceDoc) -> Result<Self> {
        if doc.schema_version != CAPS_EVIDENCE_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported evidence schema_version: {:?}",
                doc.schema_version
            );
        }

        let secure_random_bytes = decode_b64(&doc.random.secure_random_bytes_b64)?;
        let insecure_random_bytes = decode_b64(&doc.random.insecure_random_bytes_b64)?;

        let insecure_seed_u128 = if doc.random.insecure_seed_u128.is_empty() {
            0
        } else {
            doc.random
                .insecure_seed_u128
                .parse::<u128>()
                .map_err(|_| anyhow::anyhow!("invalid insecure_seed_u128"))?
        };

        Ok(Self {
            mode: CapsEvidenceMode::Replay,
            inner: Arc::new(Mutex::new(CapsEvidenceState {
                wall_clock_now_ns: doc.clocks.wall_clock_now_ns.clone(),
                monotonic_clock_now_ns: doc.clocks.monotonic_clock_now_ns.clone(),
                secure_random_bytes,
                insecure_random_bytes,
                insecure_seed_u128,
                secrets_provided: doc.secrets.provided.clone(),
                wall_clock_i: 0,
                monotonic_clock_i: 0,
                secure_random_i: 0,
                insecure_random_i: 0,
                replay_errors: Vec::new(),
            })),
        })
    }

    pub fn mode(&self) -> CapsEvidenceMode {
        self.mode
    }

    pub fn record_wall_clock_now_ns(&self, ns: u64) {
        let mut st = self.inner.lock().unwrap();
        st.wall_clock_now_ns.push(ns);
    }

    pub fn replay_wall_clock_now_ns(&self) -> Option<u64> {
        let mut st = self.inner.lock().unwrap();
        if st.wall_clock_i >= st.wall_clock_now_ns.len() {
            st.replay_errors
                .push("missing wall clock evidence".to_string());
            return None;
        }
        let out = st.wall_clock_now_ns[st.wall_clock_i];
        st.wall_clock_i += 1;
        Some(out)
    }

    pub fn record_monotonic_clock_now_ns(&self, ns: u64) {
        let mut st = self.inner.lock().unwrap();
        st.monotonic_clock_now_ns.push(ns);
    }

    pub fn replay_monotonic_clock_now_ns(&self) -> Option<u64> {
        let mut st = self.inner.lock().unwrap();
        if st.monotonic_clock_i >= st.monotonic_clock_now_ns.len() {
            st.replay_errors
                .push("missing monotonic clock evidence".to_string());
            return None;
        }
        let out = st.monotonic_clock_now_ns[st.monotonic_clock_i];
        st.monotonic_clock_i += 1;
        Some(out)
    }

    pub fn record_secure_random_bytes(&self, bytes: &[u8]) {
        let mut st = self.inner.lock().unwrap();
        st.secure_random_bytes.extend_from_slice(bytes);
    }

    pub fn replay_secure_random_bytes_into(&self, out: &mut [u8]) {
        let mut st = self.inner.lock().unwrap();
        let want = out.len();
        let remain = st
            .secure_random_bytes
            .len()
            .saturating_sub(st.secure_random_i);
        if remain < want {
            st.replay_errors
                .push("missing secure random evidence".to_string());
            out.fill(0);
            return;
        }
        let start = st.secure_random_i;
        let end = start + want;
        out.copy_from_slice(&st.secure_random_bytes[start..end]);
        st.secure_random_i = end;
    }

    pub fn record_insecure_random_bytes(&self, bytes: &[u8]) {
        let mut st = self.inner.lock().unwrap();
        st.insecure_random_bytes.extend_from_slice(bytes);
    }

    pub fn replay_insecure_random_bytes_into(&self, out: &mut [u8]) {
        let mut st = self.inner.lock().unwrap();
        let want = out.len();
        let remain = st
            .insecure_random_bytes
            .len()
            .saturating_sub(st.insecure_random_i);
        if remain < want {
            st.replay_errors
                .push("missing insecure random evidence".to_string());
            out.fill(0);
            return;
        }
        let start = st.insecure_random_i;
        let end = start + want;
        out.copy_from_slice(&st.insecure_random_bytes[start..end]);
        st.insecure_random_i = end;
    }

    pub fn set_insecure_seed_u128(&self, seed: u128) {
        let mut st = self.inner.lock().unwrap();
        st.insecure_seed_u128 = seed;
    }

    pub fn insecure_seed_u128(&self) -> u128 {
        self.inner.lock().unwrap().insecure_seed_u128
    }

    pub fn record_secret_provided(&self, id: &str, source: &str) {
        let mut st = self.inner.lock().unwrap();
        st.secrets_provided.push(CapsEvidenceSecretProvided {
            id: id.to_string(),
            source: source.to_string(),
        });
    }

    pub fn replay_errors(&self) -> Vec<String> {
        self.inner.lock().unwrap().replay_errors.clone()
    }

    pub fn build_doc(&self) -> CapsEvidenceDoc {
        let st = self.inner.lock().unwrap();
        CapsEvidenceDoc {
            schema_version: CAPS_EVIDENCE_SCHEMA_VERSION.to_string(),
            v: 1,
            clocks: CapsEvidenceClocks {
                wall_clock_now_ns: st.wall_clock_now_ns.clone(),
                monotonic_clock_now_ns: st.monotonic_clock_now_ns.clone(),
            },
            random: CapsEvidenceRandom {
                secure_random_bytes_b64: encode_b64(&st.secure_random_bytes),
                insecure_random_bytes_b64: encode_b64(&st.insecure_random_bytes),
                insecure_seed_u128: st.insecure_seed_u128.to_string(),
            },
            secrets: CapsEvidenceSecrets {
                provided: st.secrets_provided.clone(),
            },
        }
    }
}

fn encode_b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode_b64(s: &str) -> Result<Vec<u8>> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|_| anyhow::anyhow!("invalid base64"))?;
    Ok(bytes)
}
