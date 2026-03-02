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
    pub(crate) ui: DeviceProfileUi,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceProfileUi {
    pub(crate) project: PathBuf,
    pub(crate) web_ui_profile_id: String,
}
