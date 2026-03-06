use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum AppBackendAdapter {
    #[serde(rename = "wasi_http_proxy_v1")]
    WasiHttpProxyV1,
    #[serde(rename = "wasi_http_proxy_state_doc_v1")]
    WasiHttpProxyStateDocV1,
}

impl AppBackendAdapter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WasiHttpProxyV1 => "wasi_http_proxy_v1",
            Self::WasiHttpProxyStateDocV1 => "wasi_http_proxy_state_doc_v1",
        }
    }

    pub fn is_state_doc(self) -> bool {
        matches!(self, Self::WasiHttpProxyStateDocV1)
    }

    pub fn component_emit(self) -> crate::cli::ComponentBuildEmit {
        match self {
            Self::WasiHttpProxyV1 => crate::cli::ComponentBuildEmit::Http,
            Self::WasiHttpProxyStateDocV1 => crate::cli::ComponentBuildEmit::HttpStateDoc,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppBackendStateDoc {
    #[serde(default)]
    pub initial_json: Value,
}

#[derive(Debug, Clone)]
pub struct AppBackendRuntimeConfig {
    pub adapter: AppBackendAdapter,
    pub initial_state_doc: Value,
}

impl AppBackendRuntimeConfig {
    pub fn from_profile(
        adapter: AppBackendAdapter,
        state_doc: Option<&AppBackendStateDoc>,
    ) -> Self {
        Self {
            adapter,
            initial_state_doc: state_doc
                .map(|doc| doc.initial_json.clone())
                .unwrap_or(Value::Null),
        }
    }
}
