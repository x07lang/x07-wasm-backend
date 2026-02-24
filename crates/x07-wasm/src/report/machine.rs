use crate::cli::MachineArgs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonMode {
    Off,
    Canon,
    Pretty,
}

pub fn json_mode(args: &MachineArgs) -> Result<JsonMode, String> {
    let raw = if args.json.is_some() {
        args.json.as_deref()
    } else {
        args.report_json.as_deref()
    };

    let Some(raw) = raw else {
        return Ok(JsonMode::Off);
    };

    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(JsonMode::Canon);
    }
    if raw == "pretty" {
        return Ok(JsonMode::Pretty);
    }
    Err(format!(
        "unsupported --json value {raw:?}; expected \"\" or \"pretty\""
    ))
}

pub fn validate_machine_args(args: &MachineArgs) -> Option<String> {
    if let Err(err) = json_mode(args) {
        return Some(err);
    }

    let wants_json = args.json.is_some() || args.report_json.is_some();
    if !wants_json && args.report_out.is_some() {
        return Some("--report-out requires --json".to_string());
    }
    if let Some(p) = args.report_out.as_ref().and_then(|p| p.to_str()) {
        if p.trim() == "-" {
            return Some(
                "--report-out '-' is not supported (stdout is reserved for the report)".to_string(),
            );
        }
    }
    None
}
