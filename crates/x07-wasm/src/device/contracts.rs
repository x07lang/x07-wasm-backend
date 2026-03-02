use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceIndexDoc {
    pub(crate) profiles: Vec<DeviceIndexProfileRef>,
    #[serde(default)]
    pub(crate) defaults: Option<DeviceIndexDefaults>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceIndexDefaults {
    pub(crate) default_profile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceIndexProfileRef {
    pub(crate) id: String,
    pub(crate) path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceProfileDoc {
    pub(crate) id: String,
    pub(crate) v: u64,
    pub(crate) target: String,
    pub(crate) identity: DeviceProfileIdentity,
    pub(crate) version: DeviceProfileVersion,
    pub(crate) ui: DeviceProfileUi,

    #[serde(default)]
    pub(crate) desktop: Option<DeviceProfileDesktop>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceProfileIdentity {
    pub(crate) display_name: String,
    pub(crate) app_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceProfileVersion {
    pub(crate) version: String,
    pub(crate) build: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceProfileUi {
    pub(crate) project: PathBuf,
    pub(crate) web_ui_profile_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceProfileDesktop {
    pub(crate) package: DeviceProfileDesktopPackage,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceProfileDesktopPackage {
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) format: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceBundleManifestDoc {
    pub(crate) schema_version: String,
    pub(crate) kind: String,
    pub(crate) target: String,
    pub(crate) profile: DeviceBundleProfileRef,
    pub(crate) ui_wasm: DeviceBundleFileDigest,
    pub(crate) host: DeviceBundleHost,
    pub(crate) bundle_digest: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceBundleProfileRef {
    pub(crate) id: String,
    pub(crate) v: u64,
    pub(crate) file: DeviceBundleFileDigest,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceBundleFileDigest {
    pub(crate) path: String,
    pub(crate) sha256: String,
    pub(crate) bytes_len: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceBundleHost {
    pub(crate) kind: String,
    pub(crate) abi_name: String,
    pub(crate) abi_version: String,
    pub(crate) host_abi_hash: String,
}
