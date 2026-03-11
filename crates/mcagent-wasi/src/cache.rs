use std::path::{Path, PathBuf};

use mcagent_core::McAgentError;
use sha1::{Digest, Sha1};

/// Compute git blob hash: SHA1("blob <size>\0<content>").
#[must_use]
pub fn git_blob_hash(content: &[u8]) -> String {
    let header = format!("blob {}\0", content.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(content);
    hex::encode(hasher.finalize())
}

/// Get cache directory (project-local).
#[must_use]
pub fn cache_dir(project_root: &Path) -> PathBuf {
    project_root.join(".mcagent/cache/wasi")
}

/// Get cached WASM path for a given hash.
#[must_use]
pub fn cache_path(project_root: &Path, hash: &str) -> PathBuf {
    cache_dir(project_root).join(format!("{hash}.wasm"))
}

/// Check if cache is valid for the given source file.
pub fn is_cached(project_root: &Path, source: &Path) -> Result<Option<PathBuf>, McAgentError> {
    let content =
        std::fs::read(source).map_err(|e| McAgentError::filesystem(source, e))?;
    let hash = git_blob_hash(&content);
    let cached = cache_path(project_root, &hash);

    if cached.exists() {
        Ok(Some(cached))
    } else {
        Ok(None)
    }
}

/// Get the hash for a source file.
pub fn hash_source(source: &Path) -> Result<String, McAgentError> {
    let content =
        std::fs::read(source).map_err(|e| McAgentError::filesystem(source, e))?;
    Ok(git_blob_hash(&content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_blob_hash() {
        // "hello\n" should produce the well-known git blob hash.
        let hash = git_blob_hash(b"hello\n");
        assert_eq!(hash, "ce013625030ba8dba906f756967f9e9ca394464a");
    }

    #[test]
    fn test_cache_dir() {
        let dir = cache_dir(Path::new("/project"));
        assert_eq!(dir, PathBuf::from("/project/.mcagent/cache/wasi"));
    }
}
