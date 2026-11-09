use std::path::Path;

/// Normalize a filepath to be repository-relative with forward slashes,
/// stripping Windows drive letters, redundant relative segments, and multiple slashes.
pub fn normalize_rel_path(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut s = raw.as_str();

    // Strip Windows drive letter prefix (e.g. "C:/" or "c:")
    if s.len() >= 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':' {
        s = &s[2..];
    }

    // Collapse multiple consecutive slashes and normalize segments
    let mut segments = Vec::new();
    for seg in s.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => {
                segments.push(other);
            }
        }
    }

    segments.join("/")
}

/// Format a symbol URI: `path#scope`
pub fn format_symbol_uri(rel_path: &str, scope_chain: &str) -> String {
    format!("{}#{}", rel_path, scope_chain)
}

/// Parse a symbol URI into (path, scope_chain)
pub fn parse_symbol_uri(uri: &str) -> Option<(String, String)> {
    if let Some((path_part, scope_part)) = uri.rsplit_once('#') {
        if path_part.is_empty() || scope_part.is_empty() {
            return None;
        }
        let normalized_path = normalize_rel_path(Path::new(path_part));
        Some((normalized_path, scope_part.to_string()))
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
        assert_eq!(
            normalize_rel_path(Path::new("C:\\Users\\dev\\repo\\src\\main.rs")),
            "Users/dev/repo/src/main.rs"
        );
        assert_eq!(
            normalize_rel_path(Path::new("src//nested///file.rs")),
            "src/nested/file.rs"
        );
        assert_eq!(
            normalize_rel_path(Path::new("src/foo/../bar/test.ts")),
            "src/bar/test.ts"
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

    #[test]
    fn test_parse_uri_with_hash_in_path() {
        let uri = "src/c#/lexer.cs#Lexer::tokenize";
        let (p, s) = parse_symbol_uri(uri).unwrap();
        assert_eq!(p, "src/c#/lexer.cs");
        assert_eq!(s, "Lexer::tokenize");
    }

    #[test]
    fn test_parse_uri_with_backslashes() {
        let uri = "src\\auth\\token.ts#verifyToken";
        let (p, s) = parse_symbol_uri(uri).unwrap();
        assert_eq!(p, "src/auth/token.ts");
        assert_eq!(s, "verifyToken");
    }

    #[test]
    fn test_parse_invalid_uris() {
        assert!(parse_symbol_uri("no_hash_here.rs").is_none());
        assert!(parse_symbol_uri("#only_scope").is_none());
        assert!(parse_symbol_uri("only_path.rs#").is_none());
    }
}
