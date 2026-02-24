use crate::diag::{Diagnostic, Severity, Stage};

pub fn exit_code_for_diagnostics(diagnostics: &[Diagnostic]) -> u8 {
    if diagnostics.iter().all(|d| d.severity != Severity::Error) {
        return 0;
    }

    if diagnostics.iter().any(is_tool_failure) {
        return 2;
    }

    if diagnostics.iter().any(is_invalid_input) {
        return 3;
    }

    if diagnostics.iter().any(is_budget_exceeded) {
        return 4;
    }

    1
}

fn is_tool_failure(d: &Diagnostic) -> bool {
    if d.severity != Severity::Error {
        return false;
    }

    if d.code.ends_with("_SPAWN_FAILED") {
        return true;
    }
    if d.code.ends_with("_WRITE_FAILED") {
        return true;
    }
    if d.code.ends_with("_IO_FAILED") {
        return true;
    }
    if d.code.starts_with("X07WASM_INTERNAL_") {
        return true;
    }

    false
}

fn is_invalid_input(d: &Diagnostic) -> bool {
    d.severity == Severity::Error && d.stage == Stage::Parse
}

fn is_budget_exceeded(d: &Diagnostic) -> bool {
    d.severity == Severity::Error && d.code.starts_with("X07WASM_BUDGET_EXCEEDED_")
}
