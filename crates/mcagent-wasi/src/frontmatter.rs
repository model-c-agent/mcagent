use mcagent_core::{McAgentError, WasiTarget};

/// Parsed cargo frontmatter from a tool source file.
pub struct Frontmatter {
    /// Raw TOML manifest (between ---cargo and ---).
    pub manifest: String,
    /// Rust source code (after closing ---).
    pub code: String,
    /// Compilation target extracted from [package.metadata.wasi-tool].
    pub wasi_target: WasiTarget,
    /// Parsed TOML value for further inspection.
    pub parsed_manifest: toml::Value,
}

/// Parse cargo frontmatter from a tool source string.
///
/// Expected format:
/// ```text
/// #!/usr/bin/env -S cargo +nightly -Zscript
/// ---cargo
/// [package]
/// name = "example"
/// ...
/// ---
/// <rust code>
/// ```
pub fn parse(source: &str) -> Result<Frontmatter, McAgentError> {
    let lines: Vec<&str> = source.lines().collect();

    let start = lines
        .iter()
        .position(|line| line.trim() == "---cargo")
        .ok_or_else(|| McAgentError::InvalidConfig("No ---cargo frontmatter found".into()))?;

    let end = lines
        .iter()
        .skip(start + 1)
        .position(|line| line.trim() == "---")
        .ok_or_else(|| McAgentError::InvalidConfig("No closing --- for frontmatter".into()))?
        + start
        + 1;

    let manifest = lines[start + 1..end].join("\n");
    let code = lines[end + 1..].join("\n");

    let parsed_manifest: toml::Value = toml::from_str(&manifest)
        .map_err(|e| McAgentError::InvalidConfig(format!("Failed to parse manifest TOML: {e}")))?;

    let wasi_target = parsed_manifest
        .get("package")
        .and_then(|p| p.get("metadata"))
        .and_then(|m| m.get("wasi-tool"))
        .and_then(|w| w.get("wasi_target"))
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "preview2" => WasiTarget::Preview2,
            _ => WasiTarget::Preview1,
        })
        .unwrap_or_default();

    // Validate: net capability requires preview2.
    let net = parsed_manifest
        .get("package")
        .and_then(|p| p.get("metadata"))
        .and_then(|m| m.get("wasi-tool"))
        .and_then(|w| w.get("capabilities"))
        .and_then(|c| c.get("net"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);

    if net && wasi_target != WasiTarget::Preview2 {
        return Err(McAgentError::InvalidConfig(
            "net capability requires wasi_target = \"preview2\"".into(),
        ));
    }

    Ok(Frontmatter {
        manifest,
        code,
        wasi_target,
        parsed_manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_frontmatter() {
        let source = r#"#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
name = "test_tool"
edition = "2021"

[package.metadata.wasi-tool]
name = "test_tool"
wasi_target = "preview1"

[package.metadata.wasi-tool.capabilities]
read = true
write = false
net = false

[dependencies]
serde_json = "1"
---

fn main() {
    println!("hello");
}
"#;

        let fm = parse(source).unwrap();
        assert!(fm.manifest.contains("test_tool"));
        assert!(fm.code.contains("fn main()"));
        assert_eq!(fm.wasi_target, WasiTarget::Preview1);
    }

    #[test]
    fn test_net_requires_preview2() {
        let source = r#"---cargo
[package]
name = "bad_tool"
edition = "2021"

[package.metadata.wasi-tool]
wasi_target = "preview1"

[package.metadata.wasi-tool.capabilities]
net = true
---
fn main() {}
"#;

        let result = parse(source);
        assert!(result.is_err());
    }
}
