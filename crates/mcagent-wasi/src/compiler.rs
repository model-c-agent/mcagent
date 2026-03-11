use std::path::{Path, PathBuf};
use std::process::Command;

use mcagent_core::{McAgentError, WasiTarget};

use crate::{cache, frontmatter};

/// Ensure a tool is compiled, returning path to cached WASM.
///
/// Checks the cache first (by git blob hash of source), compiles if needed.
pub fn ensure_compiled(project_root: &Path, source: &Path) -> Result<PathBuf, McAgentError> {
    if let Some(cached) = cache::is_cached(project_root, source)? {
        return Ok(cached);
    }

    let hash = cache::hash_source(source)?;
    let output = cache::cache_path(project_root, &hash);

    compile_to_wasi(project_root, source, &output)?;

    Ok(output)
}

/// Compile a cargo-script tool source to WASM.
fn compile_to_wasi(
    project_root: &Path,
    source: &Path,
    output: &Path,
) -> Result<(), McAgentError> {
    // Ensure cache directory exists.
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| McAgentError::filesystem(parent, e))?;
    }

    let content = std::fs::read_to_string(source)
        .map_err(|e| McAgentError::filesystem(source, e))?;
    let fm = frontmatter::parse(&content)?;

    let temp_dir = tempfile::tempdir()
        .map_err(|e| McAgentError::CompilationFailed(format!("Failed to create temp dir: {e}")))?;
    let build_dir = temp_dir.path();

    // Read workspace Cargo.toml and replace members to point to "tool".
    let workspace_toml = std::fs::read_to_string(project_root.join("Cargo.toml"))
        .map_err(|e| McAgentError::filesystem(&project_root.join("Cargo.toml"), e))?;

    let modified_workspace = workspace_toml
        .lines()
        .map(|line| {
            if line.starts_with("members") {
                "members = [\"tool\"]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    std::fs::write(build_dir.join("Cargo.toml"), &modified_workspace)
        .map_err(|e| McAgentError::filesystem(&build_dir.join("Cargo.toml"), e))?;

    // Create tool directory with Cargo.toml and src/main.rs.
    let tool_dir = build_dir.join("tool");
    let src_dir = tool_dir.join("src");
    std::fs::create_dir_all(&src_dir)
        .map_err(|e| McAgentError::filesystem(&src_dir, e))?;

    std::fs::write(tool_dir.join("Cargo.toml"), &fm.manifest)
        .map_err(|e| McAgentError::filesystem(&tool_dir.join("Cargo.toml"), e))?;
    std::fs::write(src_dir.join("main.rs"), &fm.code)
        .map_err(|e| McAgentError::filesystem(&src_dir.join("main.rs"), e))?;

    // Symlink workspace crates so workspace dependencies resolve.
    let crates_src = project_root.join("crates");
    if crates_src.exists() {
        let crates_dst = build_dir.join("crates");
        std::os::unix::fs::symlink(&crates_src, &crates_dst).map_err(|e| {
            McAgentError::CompilationFailed(format!("Failed to symlink crates directory: {e}"))
        })?;
    }

    // Extract binary name from manifest.
    let bin_name = fm
        .manifest
        .lines()
        .find(|line| line.trim().starts_with("name = "))
        .and_then(|line| {
            let start = line.find('"')? + 1;
            let end = line.rfind('"')?;
            Some(&line[start..end])
        })
        .unwrap_or("tool");

    let (target, toolchain) = match fm.wasi_target {
        WasiTarget::Preview1 => ("wasm32-wasip1", "+nightly"),
        WasiTarget::Preview2 => ("wasm32-wasip2", "+nightly-2024-12-15"),
    };

    tracing::info!(
        tool = bin_name,
        target,
        "Compiling WASI tool"
    );

    let cargo_output = Command::new("cargo")
        .args([toolchain, "build", "--release", "--target", target, "-p", bin_name])
        .current_dir(build_dir)
        .output()
        .map_err(|e| McAgentError::CompilationFailed(format!("Failed to run cargo build: {e}")))?;

    if !cargo_output.status.success() {
        let stderr = String::from_utf8_lossy(&cargo_output.stderr);
        return Err(McAgentError::CompilationFailed(format!(
            "Compilation failed:\n{stderr}"
        )));
    }

    let wasm_source = build_dir
        .join("target")
        .join(target)
        .join("release")
        .join(format!("{bin_name}.wasm"));

    if !wasm_source.exists() {
        return Err(McAgentError::CompilationFailed(format!(
            "Compiled WASM not found at {}",
            wasm_source.display()
        )));
    }

    std::fs::copy(&wasm_source, output)
        .map_err(|e| McAgentError::filesystem(output, e))?;

    tracing::info!(
        tool = bin_name,
        output = %output.display(),
        "Tool compiled successfully"
    );

    Ok(())
}
