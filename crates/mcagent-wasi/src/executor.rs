use std::path::{Path, PathBuf};

use mcagent_core::{McAgentError, WasiTarget};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{DirPerms, FilePerms, I32Exit, WasiCtxBuilder};

/// Sandbox permissions granted to a running WASI tool.
pub struct SandboxPermissions {
    pub read_dirs: Vec<PathBuf>,
    pub write_dirs: Vec<PathBuf>,
    pub allow_net: bool,
}

/// Result of executing a WASI tool.
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run a compiled WASM module/component with the appropriate WASI runtime.
pub fn run_wasm(
    wasm_path: &Path,
    args: &[String],
    permissions: &SandboxPermissions,
    target: WasiTarget,
) -> Result<ExecutionResult, McAgentError> {
    match target {
        WasiTarget::Preview1 => run_preview1(wasm_path, args, permissions),
        WasiTarget::Preview2 => run_preview2(wasm_path, args, permissions),
    }
}

/// Run a WASI preview1 module (wasm32-wasip1).
fn run_preview1(
    wasm_path: &Path,
    args: &[String],
    permissions: &SandboxPermissions,
) -> Result<ExecutionResult, McAgentError> {
    use wasmtime::Module;
    use wasmtime_wasi::preview1::{self, WasiP1Ctx};

    struct WasiHostCtx {
        preview1: WasiP1Ctx,
        stdout: wasmtime_wasi::pipe::MemoryOutputPipe,
        stderr: wasmtime_wasi::pipe::MemoryOutputPipe,
    }

    let engine = Engine::default();
    let module = Module::from_file(&engine, wasm_path)
        .map_err(|e| McAgentError::WasiRuntime(format!("Failed to load module: {e}")))?;

    let stdout = wasmtime_wasi::pipe::MemoryOutputPipe::new(1024 * 1024);
    let stderr = wasmtime_wasi::pipe::MemoryOutputPipe::new(1024 * 1024);

    let mut wasi = WasiCtxBuilder::new();

    let mut full_args = vec![wasm_path.to_string_lossy().to_string()];
    full_args.extend(args.iter().cloned());
    wasi.args(&full_args);

    wasi.stdout(stdout.clone());
    wasi.stderr(stderr.clone());

    for dir in &permissions.read_dirs {
        if dir.exists() {
            let dir_str = dir.to_string_lossy().to_string();
            wasi.preopened_dir(dir, &dir_str, DirPerms::READ, FilePerms::READ)
                .map_err(|e| McAgentError::WasiRuntime(format!("Failed to preopen {dir_str}: {e}")))?;
        }
    }

    for dir in &permissions.write_dirs {
        if dir.exists() {
            let dir_str = dir.to_string_lossy().to_string();
            wasi.preopened_dir(dir, &dir_str, DirPerms::all(), FilePerms::all())
                .map_err(|e| McAgentError::WasiRuntime(format!("Failed to preopen {dir_str}: {e}")))?;
        }
    }

    let host_ctx = WasiHostCtx {
        preview1: wasi.build_p1(),
        stdout: stdout.clone(),
        stderr: stderr.clone(),
    };

    let mut store = Store::new(&engine, host_ctx);

    let mut linker: wasmtime::Linker<WasiHostCtx> = wasmtime::Linker::new(&engine);
    preview1::add_to_linker_sync(&mut linker, |ctx: &mut WasiHostCtx| &mut ctx.preview1)
        .map_err(|e| McAgentError::WasiRuntime(format!("Failed to link WASI: {e}")))?;

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| McAgentError::WasiRuntime(format!("Failed to instantiate: {e}")))?;

    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(|e| McAgentError::WasiRuntime(format!("No _start function: {e}")))?;

    let result = start.call(&mut store, ());

    let exit_code = match result {
        Ok(()) => 0,
        Err(e) => {
            if let Some(exit) = e.downcast_ref::<I32Exit>() {
                exit.0
            } else {
                tracing::warn!("WASM trap: {e}");
                1
            }
        }
    };

    let stdout_bytes = store.data().stdout.contents();
    let stderr_bytes = store.data().stderr.contents();

    Ok(ExecutionResult {
        exit_code,
        stdout: String::from_utf8_lossy(&stdout_bytes).to_string(),
        stderr: String::from_utf8_lossy(&stderr_bytes).to_string(),
    })
}

/// Run a WASI preview2 component (wasm32-wasip2).
fn run_preview2(
    wasm_path: &Path,
    args: &[String],
    permissions: &SandboxPermissions,
) -> Result<ExecutionResult, McAgentError> {
    use wasmtime::component::{Component, Linker, ResourceTable};
    use wasmtime_wasi::bindings::sync::Command;
    use wasmtime_wasi::{WasiCtx, WasiView};
    use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

    struct WasiHostCtx {
        ctx: WasiCtx,
        http: WasiHttpCtx,
        table: ResourceTable,
        stdout: wasmtime_wasi::pipe::MemoryOutputPipe,
        stderr: wasmtime_wasi::pipe::MemoryOutputPipe,
    }

    impl WasiView for WasiHostCtx {
        fn ctx(&mut self) -> &mut WasiCtx {
            &mut self.ctx
        }
        fn table(&mut self) -> &mut ResourceTable {
            &mut self.table
        }
    }

    impl WasiHttpView for WasiHostCtx {
        fn ctx(&mut self) -> &mut WasiHttpCtx {
            &mut self.http
        }
        fn table(&mut self) -> &mut ResourceTable {
            &mut self.table
        }
    }

    let engine = Engine::default();
    let component = Component::from_file(&engine, wasm_path)
        .map_err(|e| McAgentError::WasiRuntime(format!("Failed to load component: {e}")))?;

    let stdout = wasmtime_wasi::pipe::MemoryOutputPipe::new(1024 * 1024);
    let stderr = wasmtime_wasi::pipe::MemoryOutputPipe::new(1024 * 1024);

    let mut wasi = WasiCtxBuilder::new();

    let mut full_args = vec![wasm_path.to_string_lossy().to_string()];
    full_args.extend(args.iter().cloned());
    wasi.args(&full_args);

    wasi.stdout(stdout.clone());
    wasi.stderr(stderr.clone());

    for dir in &permissions.read_dirs {
        if dir.exists() {
            let dir_str = dir.to_string_lossy().to_string();
            wasi.preopened_dir(dir, &dir_str, DirPerms::READ, FilePerms::READ)
                .map_err(|e| McAgentError::WasiRuntime(format!("Failed to preopen {dir_str}: {e}")))?;
        }
    }

    for dir in &permissions.write_dirs {
        if dir.exists() {
            let dir_str = dir.to_string_lossy().to_string();
            wasi.preopened_dir(dir, &dir_str, DirPerms::all(), FilePerms::all())
                .map_err(|e| McAgentError::WasiRuntime(format!("Failed to preopen {dir_str}: {e}")))?;
        }
    }

    if permissions.allow_net {
        wasi.inherit_network();
        wasi.allow_ip_name_lookup(true);
    }

    let host_ctx = WasiHostCtx {
        ctx: wasi.build(),
        http: WasiHttpCtx::new(),
        table: ResourceTable::new(),
        stdout: stdout.clone(),
        stderr: stderr.clone(),
    };

    let mut store = Store::new(&engine, host_ctx);

    let mut linker: Linker<WasiHostCtx> = Linker::new(&engine);

    wasmtime_wasi::add_to_linker_sync(&mut linker)
        .map_err(|e| McAgentError::WasiRuntime(format!("Failed to link WASI: {e}")))?;
    wasmtime_wasi_http::add_only_http_to_linker_sync(&mut linker)
        .map_err(|e| McAgentError::WasiRuntime(format!("Failed to link wasi-http: {e}")))?;

    let command = Command::instantiate(&mut store, &component, &linker)
        .map_err(|e| McAgentError::WasiRuntime(format!("Failed to instantiate component: {e}")))?;

    let result = command.wasi_cli_run().call_run(&mut store);

    let exit_code = match result {
        Ok(Ok(())) => 0,
        Ok(Err(())) => 1,
        Err(e) => {
            tracing::warn!("WASM trap: {e}");
            1
        }
    };

    let stdout_bytes = store.data().stdout.contents();
    let stderr_bytes = store.data().stderr.contents();

    Ok(ExecutionResult {
        exit_code,
        stdout: String::from_utf8_lossy(&stdout_bytes).to_string(),
        stderr: String::from_utf8_lossy(&stderr_bytes).to_string(),
    })
}
