use std::convert::Infallible;
use std::path::Path;

use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::Request;

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::bindings::ProxyPre;
use wasmtime_wasi_http::body::HyperOutgoingBody;
use wasmtime_wasi_http::types::{
    default_send_request, HostFutureIncomingResponse, OutgoingRequestConfig,
};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

use crate::caps::doc::CapabilitiesDoc;
use crate::caps::enforce::build_wasi_ctx_from_caps;
use crate::diag::{Diagnostic, Severity, Stage};

#[derive(Debug, Clone, Copy)]
pub struct HttpComponentBudgets {
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_wall_ms: u64,
}

#[derive(Debug, Clone)]
pub struct BufferedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone)]
pub struct HttpComponentHost {
    engine: Engine,
    proxy_pre: ProxyPre<HostState>,
}

impl HttpComponentHost {
    pub fn from_component_file(component: &Path) -> Result<Self> {
        let mut config = Config::new();
        config.async_support(true);
        let engine = Engine::new(&config)?;

        let component = Component::from_file(&engine, component)?;

        let mut linker: Linker<HostState> = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)?;

        let proxy_pre = linker.instantiate_pre(&component)?;
        let proxy_pre = ProxyPre::new(proxy_pre)?;

        Ok(Self { engine, proxy_pre })
    }

    pub async fn handle_request<B>(
        &self,
        req: Request<B>,
        budgets: &HttpComponentBudgets,
        caps: Option<std::sync::Arc<CapabilitiesDoc>>,
        wasi_base_dir: &Path,
        request_diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<BufferedResponse>
    where
        B: http_body::Body<Data = Bytes, Error = hyper::Error> + Send + Sync + 'static,
    {
        let outgoing_body_chunk_size = budgets.max_response_bytes.clamp(1, 16 * 1024);
        let outgoing_body_buffer_chunks = budgets
            .max_response_bytes
            .div_ceil(outgoing_body_chunk_size)
            .max(1);

        let wasi = if let Some(caps) = caps.as_deref() {
            match build_wasi_ctx_from_caps(caps, wasi_base_dir, None, request_diagnostics)? {
                Some(v) => v,
                None => anyhow::bail!("capabilities denied building WASI ctx"),
            }
        } else {
            WasiCtxBuilder::new().build()
        };
        let state = HostState {
            table: ResourceTable::new(),
            wasi,
            http: WasiHttpCtx::new(),
            caps,
            diagnostics: Vec::new(),
            outgoing_body_chunk_size,
            outgoing_body_buffer_chunks,
        };

        let mut store = Store::new(&self.engine, state);
        store.data_mut().table.set_max_capacity(1024);

        let proxy = match self.proxy_pre.instantiate_async(&mut store).await {
            Ok(v) => v,
            Err(err) => {
                request_diagnostics.append(&mut store.data_mut().diagnostics);
                return Err(err);
            }
        };

        let scheme = wasmtime_wasi_http::bindings::http::types::Scheme::Http;
        let req = match store.data_mut().new_incoming_request(scheme, req) {
            Ok(v) => v,
            Err(err) => {
                request_diagnostics.append(&mut store.data_mut().diagnostics);
                return Err(err);
            }
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let out = match store.data_mut().new_response_outparam(sender) {
            Ok(v) => v,
            Err(err) => {
                request_diagnostics.append(&mut store.data_mut().diagnostics);
                return Err(err);
            }
        };

        let fut = proxy
            .wasi_http_incoming_handler()
            .call_handle(&mut store, req, out);

        let res =
            tokio::time::timeout(std::time::Duration::from_millis(budgets.max_wall_ms), fut).await;
        match res {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                request_diagnostics.append(&mut store.data_mut().diagnostics);
                return Err(anyhow::anyhow!("{err:#}"));
            }
            Err(_) => {
                request_diagnostics.append(&mut store.data_mut().diagnostics);
                return Err(anyhow::anyhow!("timeout"));
            }
        }

        let resp = match receiver.await.context("response_outparam recv") {
            Ok(v) => v,
            Err(err) => {
                request_diagnostics.append(&mut store.data_mut().diagnostics);
                return Err(err);
            }
        };
        let resp = match resp {
            Ok(v) => v,
            Err(err) => {
                request_diagnostics.append(&mut store.data_mut().diagnostics);
                return Err(err.into());
            }
        };
        let (parts, body) = resp.into_parts();
        let status = parts.status.as_u16();
        let mut headers = Vec::new();
        for (k, v) in parts.headers.iter() {
            if let Ok(v) = v.to_str() {
                headers.push((k.to_string(), v.to_string()));
            }
        }

        let body_bytes = match collect_body_with_limit(body, budgets.max_response_bytes).await {
            Ok(v) => v,
            Err(err) => {
                request_diagnostics.append(&mut store.data_mut().diagnostics);
                return Err(err);
            }
        };

        request_diagnostics.append(&mut store.data_mut().diagnostics);

        Ok(BufferedResponse {
            status,
            headers,
            body: body_bytes,
        })
    }
}

pub fn full_body(bytes: Vec<u8>) -> impl http_body::Body<Data = Bytes, Error = hyper::Error> {
    Full::new(Bytes::from(bytes)).map_err(|never: Infallible| match never {})
}

pub async fn collect_body_with_limit<B>(body: B, max_bytes: usize) -> Result<Vec<u8>>
where
    B: http_body::Body<Data = Bytes> + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let collected = Limited::new(body, max_bytes)
        .collect()
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    Ok(collected.to_bytes().to_vec())
}

struct HostState {
    table: ResourceTable,
    wasi: WasiCtx,
    http: WasiHttpCtx,
    caps: Option<std::sync::Arc<CapabilitiesDoc>>,
    diagnostics: Vec<Diagnostic>,
    outgoing_body_chunk_size: usize,
    outgoing_body_buffer_chunks: usize,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for HostState {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }

    fn outgoing_body_chunk_size(&mut self) -> usize {
        self.outgoing_body_chunk_size
    }

    fn outgoing_body_buffer_chunks(&mut self) -> usize {
        self.outgoing_body_buffer_chunks
    }

    fn send_request(
        &mut self,
        request: hyper::Request<HyperOutgoingBody>,
        config: OutgoingRequestConfig,
    ) -> wasmtime_wasi_http::HttpResult<HostFutureIncomingResponse> {
        if let Some(caps) = self.caps.as_ref() {
            let scheme = if config.use_tls { "https" } else { "http" };
            let Some(host) = request.uri().host() else {
                self.diagnostics.push(Diagnostic::new(
                    "X07WASM_CAPS_NET_DENIED",
                    Severity::Error,
                    Stage::Run,
                    "wasi:http send_request denied (missing host)".to_string(),
                ));
                return Err(
                    wasmtime_wasi_http::bindings::http::types::ErrorCode::HttpRequestDenied.into(),
                );
            };
            let port = request
                .uri()
                .port_u16()
                .unwrap_or(if config.use_tls { 443 } else { 80 });
            if !caps.network_allows(scheme, host, port) {
                self.diagnostics.push(Diagnostic::new(
                    "X07WASM_CAPS_NET_DENIED",
                    Severity::Error,
                    Stage::Run,
                    format!(
                        "wasi:http send_request denied by capabilities: {}://{}:{}",
                        scheme, host, port
                    ),
                ));
                return Err(
                    wasmtime_wasi_http::bindings::http::types::ErrorCode::HttpRequestDenied.into(),
                );
            }
        }

        Ok(default_send_request(request, config))
    }
}
