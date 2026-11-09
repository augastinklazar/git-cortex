use cortex_ast::Symbol;
use cortex_core::error::Result;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

type CacheKey = (PathBuf, [u8; 32]);

#[derive(Debug, Default)]
pub struct SymbolCache {
    entries: RwLock<HashMap<CacheKey, Vec<Symbol>>>,
}

impl SymbolCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn get_or_extract(&self, path: &Path, content: &[u8]) -> Result<Vec<Symbol>> {
        let mut hasher = Sha256::new();
        hasher.update(content);
        let hash: [u8; 32] = hasher.finalize().into();

        let key = (path.to_path_buf(), hash);

        // Read lock fast path
        if let Ok(guard) = self.entries.read() {
            if let Some(cached) = guard.get(&key) {
                return Ok(cached.clone());
            }
        }

        // Cache miss: extract symbols using tree-sitter
        let extracted = cortex_ast::extract_symbols(path, content)?;

        // Write lock insertion
        if let Ok(mut guard) = self.entries.write() {
            guard.insert(key, extracted.clone());
        }

        Ok(extracted)
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.entries.write() {
            guard.clear();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.read().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_cache_memoization() {
        let cache = SymbolCache::new();
        let path = Path::new("src/math.rs");
        let code = b"pub fn add(a: i32, b: i32) -> i32 { a + b }\n";

        assert_eq!(cache.len(), 0);
        let syms1 = cache.get_or_extract(path, code).unwrap();
        assert_eq!(syms1.len(), 1);
        assert_eq!(cache.len(), 1);

        // Second call with same content should hit cache
        let syms2 = cache.get_or_extract(path, code).unwrap();
        assert_eq!(syms2.len(), 1);
        assert_eq!(syms1[0].ast_hash, syms2[0].ast_hash);
        assert_eq!(cache.len(), 1);

        // Modified content should add a new cache entry
        let code_modified = b"pub fn add(a: i32, b: i32) -> i32 { a + b + 1 }\n";
        let syms3 = cache.get_or_extract(path, code_modified).unwrap();
        assert_eq!(cache.len(), 2);
        assert_ne!(syms1[0].ast_hash, syms3[0].ast_hash);

        cache.clear();
        assert_eq!(cache.len(), 0);
    }
}
