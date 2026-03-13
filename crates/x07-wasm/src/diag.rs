use serde::Serialize;

const DIAGNOSTIC_MESSAGE_MAX_CHARS: usize = 4096;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Parse,
    Lint,
    Rewrite,
    Type,
    Lower,
    Codegen,
    Link,
    Run,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub stage: Stage,
    pub message: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<serde_json::Value>,

    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub data: std::collections::BTreeMap<String, serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub quickfix: Option<serde_json::Value>,
}

impl Diagnostic {
    pub fn new(
        code: impl Into<String>,
        severity: Severity,
        stage: Stage,
        message: impl Into<String>,
    ) -> Self {
        let mut message = message.into();
        if message.chars().nth(DIAGNOSTIC_MESSAGE_MAX_CHARS).is_some() {
            message = message.chars().take(DIAGNOSTIC_MESSAGE_MAX_CHARS).collect();
        }
        Self {
            code: code.into(),
            severity,
            stage,
            message,
            loc: None,
            notes: Vec::new(),
            related: Vec::new(),
            data: std::collections::BTreeMap::new(),
            quickfix: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_overlong_messages_to_schema_limit() {
        let diagnostic = Diagnostic::new(
            "X07WASM_TEST",
            Severity::Error,
            Stage::Run,
            "x".repeat(DIAGNOSTIC_MESSAGE_MAX_CHARS + 64),
        );

        assert_eq!(diagnostic.message.chars().count(), DIAGNOSTIC_MESSAGE_MAX_CHARS);
    }
}
