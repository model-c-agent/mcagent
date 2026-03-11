use std::path::{Path, PathBuf};

use mcagent_core::{
    ArgSpec, ErrorSpec, McAgentError, ToolCapabilities, ToolMetadata, WasiTarget,
};

use crate::executor::{self, SandboxPermissions};
use crate::frontmatter;

/// Extract metadata from a compiled WASM tool by running it with --describe.
pub fn extract(wasm_path: &Path, target: WasiTarget) -> Result<ToolMetadata, McAgentError> {
    let permissions = SandboxPermissions {
        read_dirs: Vec::new(),
        write_dirs: Vec::new(),
        allow_net: false,
    };

    let result = executor::run_wasm(wasm_path, &["--describe".to_string()], &permissions, target)?;

    if result.exit_code != 0 {
        return Err(McAgentError::WasiRuntime(format!(
            "Tool --describe failed with code {}: {}",
            result.exit_code, result.stderr
        )));
    }

    serde_json::from_str(&result.stdout).map_err(|e| {
        McAgentError::WasiRuntime(format!("Failed to parse tool metadata: {e}"))
    })
}

/// Extract metadata from a source file's cargo frontmatter.
pub fn extract_from_source(source: &Path) -> Result<ToolMetadata, McAgentError> {
    let content = std::fs::read_to_string(source)
        .map_err(|e| McAgentError::filesystem(source, e))?;
    extract_from_frontmatter(&content)
}

/// Extract just the WasiTarget from a source file's frontmatter.
pub fn extract_target_from_source(source: &Path) -> Result<WasiTarget, McAgentError> {
    let content = std::fs::read_to_string(source)
        .map_err(|e| McAgentError::filesystem(source, e))?;
    let fm = frontmatter::parse(&content)?;
    Ok(fm.wasi_target)
}

/// List all .rs tool source files in a directory.
pub fn list_source_tools(dir: &Path) -> Result<Vec<PathBuf>, McAgentError> {
    let mut tools = Vec::new();

    if !dir.exists() {
        return Ok(tools);
    }

    let entries = std::fs::read_dir(dir)
        .map_err(|e| McAgentError::filesystem(dir, e))?;

    for entry in entries {
        let entry = entry.map_err(|e| McAgentError::filesystem(dir, e))?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            tools.push(path);
        }
    }

    tools.sort();
    Ok(tools)
}

/// Parse metadata from cargo frontmatter content.
fn extract_from_frontmatter(source: &str) -> Result<ToolMetadata, McAgentError> {
    let fm = frontmatter::parse(source)?;

    let metadata = fm
        .parsed_manifest
        .get("package")
        .and_then(|p| p.get("metadata"))
        .and_then(|m| m.get("wasi-tool"))
        .ok_or_else(|| {
            McAgentError::InvalidConfig("No [package.metadata.wasi-tool] section found".into())
        })?;

    let name = metadata
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McAgentError::InvalidConfig("Missing 'name' in wasi-tool metadata".into()))?
        .to_string();

    let version = metadata
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();

    let description = metadata
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let caps = metadata.get("capabilities");
    let capabilities = ToolCapabilities {
        read: caps
            .and_then(|c| c.get("read"))
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
        write: caps
            .and_then(|c| c.get("write"))
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
        net: caps
            .and_then(|c| c.get("net"))
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
    };

    let args = metadata
        .get("args")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|arg| {
                    Some(ArgSpec {
                        name: arg.get("name")?.as_str()?.to_string(),
                        arg_type: arg.get("type")?.as_str()?.to_string(),
                        description: arg
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        required: arg
                            .get("required")
                            .and_then(toml::Value::as_bool)
                            .unwrap_or(false),
                        default: arg
                            .get("default")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let errors = metadata
        .get("errors")
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|err| {
                    #[allow(clippy::cast_possible_truncation)]
                    Some(ErrorSpec {
                        code: err.get("code")?.as_integer()? as i32,
                        message: err.get("message")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ToolMetadata {
        name,
        version,
        description,
        args,
        errors,
        capabilities,
        wasi_target: fm.wasi_target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_from_frontmatter() {
        let source = r#"#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
name = "test_tool"
edition = "2021"

[package.metadata.wasi-tool]
name = "test_tool"
version = "1.0.0"
description = "A test tool"

[package.metadata.wasi-tool.capabilities]
read = true
write = false
net = false

[[package.metadata.wasi-tool.args]]
name = "input"
type = "string"
description = "Input file"
required = true

[[package.metadata.wasi-tool.errors]]
code = 100
message = "Custom error"

[dependencies]
serde_json = "1"
---

fn main() {}
"#;

        let metadata = extract_from_frontmatter(source).unwrap();
        assert_eq!(metadata.name, "test_tool");
        assert_eq!(metadata.version, "1.0.0");
        assert_eq!(metadata.description, "A test tool");
        assert!(metadata.capabilities.read);
        assert!(!metadata.capabilities.write);
        assert!(!metadata.capabilities.net);
        assert_eq!(metadata.wasi_target, WasiTarget::Preview1);
        assert_eq!(metadata.args.len(), 1);
        assert_eq!(metadata.args[0].name, "input");
        assert!(metadata.args[0].required);
        assert_eq!(metadata.errors.len(), 1);
        assert_eq!(metadata.errors[0].code, 100);
    }
}
