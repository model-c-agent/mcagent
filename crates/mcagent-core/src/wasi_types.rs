use serde::{Deserialize, Serialize};

/// Standard exit codes for WASI tools (0-99 reserved, 100+ tool-specific).
pub mod exit_codes {
    pub const SUCCESS: i32 = 0;
    pub const INVALID_ARGS: i32 = 1;
    pub const FILE_NOT_FOUND: i32 = 2;
    pub const PERMISSION_DENIED: i32 = 3;
    pub const NETWORK_ERROR: i32 = 4;
    pub const PARSE_ERROR: i32 = 5;
    pub const TIMEOUT: i32 = 6;
    pub const INTERNAL_ERROR: i32 = 99;
    /// Tool-specific errors start at 100.
    pub const TOOL_SPECIFIC_START: i32 = 100;
}

/// WASI compilation/execution target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WasiTarget {
    /// WASI preview1 — simpler, full exit code support, no networking.
    #[default]
    Preview1,
    /// WASI preview2 — networking support via wasi-http, exit codes limited to 0/1.
    Preview2,
}

/// Declared capabilities a tool requires (from frontmatter metadata).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolCapabilities {
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub net: bool,
}

/// Tool metadata, extractable from frontmatter or --describe output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub args: Vec<ArgSpec>,
    pub errors: Vec<ErrorSpec>,
    pub capabilities: ToolCapabilities,
    #[serde(default)]
    pub wasi_target: WasiTarget,
}

/// Specification for a tool argument.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub arg_type: String,
    pub description: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// Specification for a tool error code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorSpec {
    pub code: i32,
    pub message: String,
}

/// Helper: print metadata JSON to stdout (used inside WASI tool binaries).
pub fn print_metadata(metadata: &ToolMetadata) {
    println!(
        "{}",
        serde_json::to_string(metadata).expect("metadata serialization")
    );
}
