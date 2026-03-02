use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

pub const IN_TOTO_STATEMENT_PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DsseEnvelope {
    #[serde(rename = "payloadType")]
    pub payload_type: String,
    pub payload: String,
    pub signatures: Vec<DsseSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DsseSignature {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyid: Option<String>,
    pub sig: String,
}

pub fn pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let payload_type_len = payload_type.len();
    let payload_len = payload.len();
    let mut out = Vec::with_capacity(6 + 1 + 20 + 1 + payload_type_len + 1 + 20 + 1 + payload_len);
    out.extend_from_slice(b"DSSEv1");
    out.extend_from_slice(b" ");
    out.extend_from_slice(payload_type_len.to_string().as_bytes());
    out.extend_from_slice(b" ");
    out.extend_from_slice(payload_type.as_bytes());
    out.extend_from_slice(b" ");
    out.extend_from_slice(payload_len.to_string().as_bytes());
    out.extend_from_slice(b" ");
    out.extend_from_slice(payload);
    out
}

pub fn sign_ed25519_envelope(
    payload_type: &str,
    payload: &[u8],
    signing_key: &SigningKey,
    keyid: Option<String>,
) -> DsseEnvelope {
    use ed25519_dalek::Signer as _;

    let message = pae(payload_type, payload);
    let sig: Signature = signing_key.sign(&message);
    let sig_b64 = STANDARD.encode(sig.to_bytes());

    DsseEnvelope {
        payload_type: payload_type.to_string(),
        payload: STANDARD.encode(payload),
        signatures: vec![DsseSignature {
            keyid,
            sig: sig_b64,
        }],
    }
}

pub fn verify_ed25519_signature(
    envelope: &DsseEnvelope,
    trusted_public_key: &VerifyingKey,
) -> Result<(), ()> {
    use ed25519_dalek::Verifier as _;

    let payload = STANDARD.decode(&envelope.payload).map_err(|_| ())?;
    let Some(sig0) = envelope.signatures.first() else {
        return Err(());
    };
    let sig_bytes = STANDARD.decode(&sig0.sig).map_err(|_| ())?;
    let sig_bytes: [u8; 64] = sig_bytes.try_into().map_err(|_| ())?;
    let sig = Signature::from_bytes(&sig_bytes);

    let message = pae(&envelope.payload_type, &payload);
    trusted_public_key.verify(&message, &sig).map_err(|_| ())
}

pub fn decode_payload(envelope: &DsseEnvelope) -> Result<Vec<u8>, ()> {
    STANDARD.decode(&envelope.payload).map_err(|_| ())
}
