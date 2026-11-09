use std::path::Path;

/// Normalize a filepath to be repository-relative with forward slashes
pub fn normalize_rel_path(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let trimmed = s.trim_start_matches("./");
    trimmed.trim_start_matches('/').to_string()
}

/// Format a symbol URI: `path#scope`
pub fn format_symbol_uri(rel_path: &str, scope_chain: &str) -> String {
    format!("{}#{}", rel_path, scope_chain)
}

/// Parse a symbol URI into (path, scope_chain)
pub fn parse_symbol_uri(uri: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = uri.splitn(2, '#').collect();
    if parts.len() == 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(
            normalize_rel_path(Path::new("src\\auth\\token.ts")),
            "src/auth/token.ts"
        );
        assert_eq!(
            normalize_rel_path(Path::new("./crates/lib/src/main.rs")),
            "crates/lib/src/main.rs"
        );
    }

    #[test]
    fn test_format_and_parse_uri() {
        let uri = format_symbol_uri("src/pool/connection.rs", "ConnectionPool::acquire");
        assert_eq!(uri, "src/pool/connection.rs#ConnectionPool::acquire");

        let (p, s) = parse_symbol_uri(&uri).unwrap();
        assert_eq!(p, "src/pool/connection.rs");
        assert_eq!(s, "ConnectionPool::acquire");
    }
}
