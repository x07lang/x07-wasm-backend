use serde::Serialize;

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
        Self {
            code: code.into(),
            severity,
            stage,
            message: message.into(),
            loc: None,
            notes: Vec::new(),
            related: Vec::new(),
            data: std::collections::BTreeMap::new(),
            quickfix: None,
        }
    }
}
