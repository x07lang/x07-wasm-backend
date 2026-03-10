use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct WebUiRuntimeManifest {
    #[serde(rename = "arenaCapBytes")]
    pub(crate) arena_cap_bytes: u64,
    #[serde(rename = "maxInputBytes")]
    pub(crate) max_input_bytes: u64,
    #[serde(rename = "maxOutputBytes")]
    pub(crate) max_output_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct WebUiProfileDoc {
    defaults: WebUiProfileDefaults,
}

#[derive(Debug, Deserialize)]
struct WebUiProfileDefaults {
    arena_cap_bytes: u64,
    max_input_bytes: u64,
    max_output_bytes: u64,
}

pub(crate) fn load_runtime_manifest_from_profile(
    profile_path: &Path,
) -> Result<WebUiRuntimeManifest> {
    let bytes = std::fs::read(profile_path)
        .with_context(|| format!("read web-ui profile {}", profile_path.display()))?;
    let doc: WebUiProfileDoc = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse web-ui profile {}", profile_path.display()))?;
    Ok(WebUiRuntimeManifest {
        arena_cap_bytes: doc.defaults.arena_cap_bytes,
        max_input_bytes: doc.defaults.max_input_bytes,
        max_output_bytes: doc.defaults.max_output_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_runtime_defaults_from_profile_doc() {
        let doc = br#"{
          "schema_version": "x07.web_ui.profile@0.1.0",
          "id": "web_ui_debug",
          "v": 1,
          "defaults": {
            "arena_cap_bytes": 50331648,
            "max_input_bytes": 65536,
            "max_output_bytes": 2097152
          }
        }"#;
        let parsed: WebUiProfileDoc = serde_json::from_slice(doc).expect("parse");
        let runtime = WebUiRuntimeManifest {
            arena_cap_bytes: parsed.defaults.arena_cap_bytes,
            max_input_bytes: parsed.defaults.max_input_bytes,
            max_output_bytes: parsed.defaults.max_output_bytes,
        };
        assert_eq!(
            runtime,
            WebUiRuntimeManifest {
                arena_cap_bytes: 50_331_648,
                max_input_bytes: 65_536,
                max_output_bytes: 2_097_152,
            }
        );
    }
}
