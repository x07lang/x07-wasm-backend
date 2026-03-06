use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use base64::Engine as _;
use bytes::Bytes;
use hyper::header::{HeaderName, HeaderValue};
use hyper::Request;
use serde_json::Value;

use crate::app::backend::{AppBackendAdapter, AppBackendRuntimeConfig};
use crate::caps::doc::CapabilitiesDoc;
use crate::diag::{Diagnostic, Severity, Stage};
use crate::http_component_host::{BufferedResponse, HttpComponentBudgets, HttpComponentHost};
use crate::report;

const X07_STATE_DOC_REQUEST_HEADER: &str = "x-x07-state-doc-b64";
const X07_STATE_DOC_RESPONSE_HEADER: &str = "x-x07-next-state-doc-b64";

#[derive(Clone)]
pub struct AppBackendHost {
    adapter: AppBackendAdapter,
    host: HttpComponentHost,
    state_doc: Arc<tokio::sync::Mutex<Value>>,
}

impl AppBackendHost {
    pub fn from_component_file(
        component: &Path,
        backend: AppBackendRuntimeConfig,
        runtime_limits: crate::arch::WasmRuntimeLimits,
        max_concurrency: usize,
    ) -> Result<Self> {
        let host =
            HttpComponentHost::from_component_file(component, runtime_limits, max_concurrency)
                .with_context(|| format!("load backend component {}", component.display()))?;
        Ok(Self {
            adapter: backend.adapter,
            host,
            state_doc: Arc::new(tokio::sync::Mutex::new(backend.initial_state_doc)),
        })
    }

    pub async fn handle_request<B>(
        &self,
        req: Request<B>,
        budgets: &HttpComponentBudgets,
        caps: Option<Arc<CapabilitiesDoc>>,
        wasi_base_dir: &Path,
        request_diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<BufferedResponse>
    where
        B: http_body::Body<Data = Bytes, Error = hyper::Error> + Send + Sync + 'static,
    {
        if !self.adapter.is_state_doc() {
            return self
                .host
                .handle_request(req, budgets, caps, wasi_base_dir, request_diagnostics)
                .await;
        }

        let mut state_doc = self.state_doc.lock().await;
        let req = match inject_state_doc_header(req, &state_doc) {
            Ok(v) => v,
            Err(err) => {
                request_diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_BACKEND_STATE_DOC_REQUEST_INVALID",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to encode backend state doc: {err:#}"),
                ));
                return Err(err);
            }
        };

        let mut resp = self
            .host
            .handle_request(req, budgets, caps, wasi_base_dir, request_diagnostics)
            .await?;

        match take_response_state_doc(&mut resp.headers) {
            Ok(next_state_doc) => {
                *state_doc = next_state_doc;
                Ok(resp)
            }
            Err(err) => {
                request_diagnostics.push(Diagnostic::new(
                    "X07WASM_APP_BACKEND_STATE_DOC_RESPONSE_INVALID",
                    Severity::Error,
                    Stage::Run,
                    format!("failed to decode backend state doc response: {err:#}"),
                ));
                Err(err)
            }
        }
    }
}

fn inject_state_doc_header<B>(req: Request<B>, state_doc: &Value) -> Result<Request<B>> {
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(report::canon::canonical_json_bytes(state_doc)?);
    let value = HeaderValue::from_str(&encoded).context("build state-doc header value")?;

    let (mut parts, body) = req.into_parts();
    parts
        .headers
        .insert(HeaderName::from_static(X07_STATE_DOC_REQUEST_HEADER), value);
    Ok(Request::from_parts(parts, body))
}

fn take_response_state_doc(headers: &mut Vec<(String, String)>) -> Result<Value> {
    let mut next_state_doc_b64: Option<String> = None;
    let mut duplicate = false;
    headers.retain(|(k, v)| {
        if k.eq_ignore_ascii_case(X07_STATE_DOC_RESPONSE_HEADER) {
            if next_state_doc_b64.replace(v.clone()).is_some() {
                duplicate = true;
            }
            false
        } else {
            true
        }
    });

    if duplicate {
        anyhow::bail!("duplicate response header: {X07_STATE_DOC_RESPONSE_HEADER}");
    }
    let Some(next_state_doc_b64) = next_state_doc_b64 else {
        anyhow::bail!("missing response header: {X07_STATE_DOC_RESPONSE_HEADER}");
    };

    let next_state_doc_bytes = base64::engine::general_purpose::STANDARD
        .decode(next_state_doc_b64.as_bytes())
        .context("base64 decode next state doc")?;
    let next_state_doc: Value =
        serde_json::from_slice(&next_state_doc_bytes).context("parse next state doc JSON")?;
    Ok(next_state_doc)
}
