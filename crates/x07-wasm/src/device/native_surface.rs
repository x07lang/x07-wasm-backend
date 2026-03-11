use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeSurface {
    pub(crate) native_summary: NativeSummary,
    pub(crate) release_readiness: ReleaseReadiness,
}

#[derive(Debug, Clone)]
pub(crate) struct DeriveNativeSurfaceArgs<'a> {
    pub(crate) target_kind: &'a str,
    pub(crate) bundle_manifest_sha256: Option<&'a str>,
    pub(crate) package_manifest_sha256: Option<&'a str>,
    pub(crate) capabilities_doc: &'a Value,
    pub(crate) telemetry_profile_doc: &'a Value,
    pub(crate) extra_warnings: Vec<ReadinessIssue>,
    pub(crate) extra_errors: Vec<ReadinessIssue>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeSummary {
    pub(crate) target_kind: String,
    pub(crate) provider_kind: Option<String>,
    pub(crate) bundle_manifest_sha256: Option<String>,
    pub(crate) package_manifest_sha256: Option<String>,
    pub(crate) capabilities: NativeCapabilitiesSummary,
    pub(crate) permission_declarations: Vec<String>,
    pub(crate) telemetry_classes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeCapabilitiesSummary {
    pub(crate) camera_photo: bool,
    pub(crate) audio_playback: bool,
    pub(crate) files_pick: bool,
    pub(crate) files_pick_multiple: bool,
    pub(crate) files_save: bool,
    pub(crate) files_drop: bool,
    pub(crate) files_accept_defaults: Vec<String>,
    pub(crate) clipboard_read_text: bool,
    pub(crate) clipboard_write_text: bool,
    pub(crate) blob_store: BlobStoreSummary,
    pub(crate) haptics_present: bool,
    pub(crate) location_foreground: bool,
    pub(crate) notifications_local: bool,
    pub(crate) notifications_push: bool,
    pub(crate) share_present: bool,
    pub(crate) network_allow_hosts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct BlobStoreSummary {
    pub(crate) enabled: bool,
    pub(crate) max_total_bytes: u64,
    pub(crate) max_item_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReleaseReadiness {
    pub(crate) status: String,
    pub(crate) warnings: Vec<ReadinessIssue>,
    pub(crate) errors: Vec<ReadinessIssue>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReadinessIssue {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) field: Option<String>,
}

impl ReadinessIssue {
    pub(crate) fn new(code: &str, message: impl Into<String>, field: Option<&str>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            field: field.map(str::to_string),
        }
    }
}

pub(crate) fn derive_native_surface(args: DeriveNativeSurfaceArgs<'_>) -> NativeSurface {
    let capabilities = derive_capabilities_summary(args.capabilities_doc);
    let telemetry_classes = telemetry_classes(args.telemetry_profile_doc);
    let mut warnings = args.extra_warnings;
    let mut errors = args.extra_errors;

    if capabilities.notifications_push {
        errors.push(ReadinessIssue::new(
            "X07WASM_DEVICE_NATIVE_PUSH_NOT_SUPPORTED",
            "device capabilities request push notifications, but the strict-M1 native surface only supports local notifications",
            Some("/device/notifications/push"),
        ));
    }

    if let Some(resource_target) = args
        .telemetry_profile_doc
        .pointer("/resource/target")
        .and_then(Value::as_str)
    {
        if resource_target != args.target_kind {
            errors.push(ReadinessIssue::new(
                "X07WASM_DEVICE_NATIVE_TELEMETRY_TARGET_MISMATCH",
                format!(
                    "telemetry resource target {:?} does not match device target {:?}",
                    resource_target, args.target_kind
                ),
                Some("/resource/target"),
            ));
        }
    }

    if let Some(endpoint) = args
        .telemetry_profile_doc
        .pointer("/transport/endpoint")
        .and_then(Value::as_str)
    {
        if endpoint.starts_with("http://") && !is_local_http_endpoint(endpoint) {
            warnings.push(ReadinessIssue::new(
                "X07WASM_DEVICE_NATIVE_TELEMETRY_ENDPOINT_INSECURE",
                format!(
                    "telemetry endpoint {:?} uses plain HTTP outside localhost/127.0.0.1",
                    endpoint
                ),
                Some("/transport/endpoint"),
            ));
        }
    }

    let status = if !errors.is_empty() {
        "error"
    } else if !warnings.is_empty() {
        "warning"
    } else {
        "ok"
    };

    NativeSurface {
        native_summary: NativeSummary {
            target_kind: args.target_kind.to_string(),
            provider_kind: None,
            bundle_manifest_sha256: args.bundle_manifest_sha256.map(str::to_string),
            package_manifest_sha256: args.package_manifest_sha256.map(str::to_string),
            capabilities: capabilities.clone(),
            permission_declarations: permission_declarations(&capabilities),
            telemetry_classes,
        },
        release_readiness: ReleaseReadiness {
            status: status.to_string(),
            warnings,
            errors,
        },
    }
}

pub(crate) fn android_runtime_permissions(capabilities: &Value) -> Vec<String> {
    let mut permissions: Vec<String> = Vec::new();

    if capability_bool(capabilities, "/device/camera/photo") {
        permissions.push("android.permission.CAMERA".to_string());
    }
    if capability_bool(capabilities, "/device/haptics/present") {
        permissions.push("android.permission.VIBRATE".to_string());
    }
    if capability_bool(capabilities, "/device/location/foreground") {
        permissions.push("android.permission.ACCESS_COARSE_LOCATION".to_string());
        permissions.push("android.permission.ACCESS_FINE_LOCATION".to_string());
    }
    if capability_bool(capabilities, "/device/notifications/local") {
        permissions.push("android.permission.POST_NOTIFICATIONS".to_string());
    }
    if capability_files_pick_requested(capabilities)
        && capability_accept_defaults(capabilities)
            .iter()
            .any(|value| value == "image/*")
    {
        permissions.push("android.permission.READ_MEDIA_IMAGES".to_string());
    }

    permissions.sort();
    permissions.dedup();
    permissions
}

pub(crate) fn ios_usage_descriptions(
    display_name: &str,
    capabilities: &Value,
) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = Vec::new();

    if capability_bool(capabilities, "/device/camera/photo") {
        entries.push((
            "NSCameraUsageDescription".to_string(),
            format!("{display_name} uses the camera to capture photos you choose."),
        ));
    }
    if capability_files_pick_requested(capabilities) {
        entries.push((
            "NSPhotoLibraryUsageDescription".to_string(),
            format!("{display_name} imports photos and documents that you choose."),
        ));
    }
    if capability_bool(capabilities, "/device/location/foreground") {
        entries.push((
            "NSLocationWhenInUseUsageDescription".to_string(),
            format!("{display_name} uses your location while the app is open."),
        ));
    }

    entries
}

fn derive_capabilities_summary(doc: &Value) -> NativeCapabilitiesSummary {
    NativeCapabilitiesSummary {
        camera_photo: capability_bool(doc, "/device/camera/photo"),
        audio_playback: capability_bool(doc, "/device/audio/playback"),
        files_pick: capability_bool(doc, "/device/files/pick"),
        files_pick_multiple: capability_bool(doc, "/device/files/pick_multiple"),
        files_save: capability_bool(doc, "/device/files/save"),
        files_drop: capability_bool(doc, "/device/files/drop"),
        files_accept_defaults: capability_accept_defaults(doc),
        clipboard_read_text: capability_bool(doc, "/device/clipboard/read_text"),
        clipboard_write_text: capability_bool(doc, "/device/clipboard/write_text"),
        blob_store: BlobStoreSummary {
            enabled: capability_bool(doc, "/device/blob_store/enabled"),
            max_total_bytes: doc
                .pointer("/device/blob_store/max_total_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            max_item_bytes: doc
                .pointer("/device/blob_store/max_item_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        },
        haptics_present: capability_bool(doc, "/device/haptics/present"),
        location_foreground: capability_bool(doc, "/device/location/foreground"),
        notifications_local: capability_bool(doc, "/device/notifications/local"),
        notifications_push: capability_bool(doc, "/device/notifications/push"),
        share_present: capability_bool(doc, "/device/share/present"),
        network_allow_hosts: doc
            .pointer("/network/allow_hosts")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    }
}

fn permission_declarations(capabilities: &NativeCapabilitiesSummary) -> Vec<String> {
    let mut permissions = Vec::new();

    if capabilities.camera_photo {
        permissions.push("camera".to_string());
    }
    if capabilities.files_pick || capabilities.files_pick_multiple {
        permissions.push("files_pick".to_string());
    }
    if capabilities.haptics_present {
        permissions.push("haptics_present".to_string());
    }
    if capabilities.location_foreground {
        permissions.push("location_foreground".to_string());
    }
    if capabilities.notifications_local {
        permissions.push("notifications_local".to_string());
    }
    if capabilities.notifications_push {
        permissions.push("notifications_push".to_string());
    }

    permissions
}

fn telemetry_classes(doc: &Value) -> Vec<String> {
    doc.pointer("/event_classes")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn capability_bool(doc: &Value, pointer: &str) -> bool {
    doc.pointer(pointer)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn capability_files_pick_requested(doc: &Value) -> bool {
    capability_bool(doc, "/device/files/pick")
        || capability_bool(doc, "/device/files/pick_multiple")
}

fn capability_accept_defaults(doc: &Value) -> Vec<String> {
    doc.pointer("/device/files/accept_defaults")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn is_local_http_endpoint(endpoint: &str) -> bool {
    endpoint.starts_with("http://localhost")
        || endpoint.starts_with("http://127.0.0.1")
        || endpoint.starts_with("http://[::1]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn capability_doc(push: bool) -> Value {
        json!({
            "schema_version": "x07.device.capabilities@0.2.0",
            "network": {
                "mode": "deny_by_default",
                "allow_hosts": ["api.example.com"]
            },
            "device": {
                "camera": { "photo": true },
                "audio": { "playback": true },
                "files": {
                    "pick": true,
                    "pick_multiple": true,
                    "save": true,
                    "drop": true,
                    "accept_defaults": ["image/*", "application/pdf"]
                },
                "clipboard": {
                    "read_text": true,
                    "write_text": true
                },
                "blob_store": {
                    "enabled": true,
                    "max_total_bytes": 67108864,
                    "max_item_bytes": 16777216
                },
                "haptics": { "present": true },
                "location": { "foreground": true },
                "notifications": { "local": true, "push": push },
                "share": { "present": true }
            }
        })
    }

    fn telemetry_doc(endpoint: &str) -> Value {
        json!({
            "schema_version": "x07.device.telemetry.profile@0.1.0",
            "transport": {
                "protocol": "http/protobuf",
                "endpoint": endpoint
            },
            "resource": {
                "target": "ios"
            },
            "event_classes": [
                "app.lifecycle",
                "app.http",
                "runtime.error",
                "bridge.timing",
                "reducer.timing",
                "policy.violation",
                "host.webview_crash"
            ]
        })
    }

    #[test]
    fn derive_native_surface_reports_unsupported_push() {
        let derived = derive_native_surface(DeriveNativeSurfaceArgs {
            target_kind: "ios",
            bundle_manifest_sha256: Some("a"),
            package_manifest_sha256: Some("b"),
            capabilities_doc: &capability_doc(true),
            telemetry_profile_doc: &telemetry_doc("https://otel.example.invalid:4318"),
            extra_warnings: Vec::new(),
            extra_errors: Vec::new(),
        });

        assert_eq!(derived.release_readiness.status, "error");
        assert!(derived
            .release_readiness
            .errors
            .iter()
            .any(|issue| issue.code == "X07WASM_DEVICE_NATIVE_PUSH_NOT_SUPPORTED"));
    }

    #[test]
    fn ios_usage_descriptions_are_app_specific() {
        let entries = ios_usage_descriptions("Field Notes", &capability_doc(false));
        let body = entries
            .iter()
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(body.contains("Field Notes"));
        assert!(!body.contains("CrewOps"));
    }

    #[test]
    fn derive_native_surface_warns_on_insecure_non_local_telemetry() {
        let derived = derive_native_surface(DeriveNativeSurfaceArgs {
            target_kind: "ios",
            bundle_manifest_sha256: Some("a"),
            package_manifest_sha256: Some("b"),
            capabilities_doc: &capability_doc(false),
            telemetry_profile_doc: &telemetry_doc("http://otel.example.invalid:4318"),
            extra_warnings: Vec::new(),
            extra_errors: Vec::new(),
        });

        assert_eq!(derived.release_readiness.status, "warning");
        assert!(derived
            .release_readiness
            .warnings
            .iter()
            .any(|issue| issue.code == "X07WASM_DEVICE_NATIVE_TELEMETRY_ENDPOINT_INSECURE"));
    }

    #[test]
    fn derive_native_surface_carries_builder_io_capabilities() {
        let derived = derive_native_surface(DeriveNativeSurfaceArgs {
            target_kind: "ios",
            bundle_manifest_sha256: Some("a"),
            package_manifest_sha256: Some("b"),
            capabilities_doc: &capability_doc(false),
            telemetry_profile_doc: &telemetry_doc("https://otel.example.invalid:4318"),
            extra_warnings: Vec::new(),
            extra_errors: Vec::new(),
        });

        let capabilities = &derived.native_summary.capabilities;
        assert!(capabilities.files_pick);
        assert!(capabilities.files_pick_multiple);
        assert!(capabilities.files_save);
        assert!(capabilities.files_drop);
        assert!(capabilities.audio_playback);
        assert!(capabilities.clipboard_read_text);
        assert!(capabilities.clipboard_write_text);
        assert!(capabilities.haptics_present);
        assert!(capabilities.share_present);
        assert!(derived
            .native_summary
            .permission_declarations
            .iter()
            .any(|entry| entry == "files_pick"));
        assert!(derived
            .native_summary
            .permission_declarations
            .iter()
            .any(|entry| entry == "haptics_present"));
    }

    #[test]
    fn android_runtime_permissions_include_haptics_vibrate() {
        let permissions = android_runtime_permissions(&capability_doc(false));
        assert!(permissions.iter().any(|entry| entry == "android.permission.VIBRATE"));
    }
}
