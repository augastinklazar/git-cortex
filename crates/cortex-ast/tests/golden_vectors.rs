use cortex_ast::*;
use std::path::Path;

#[test]
fn test_typescript_golden_and_comment_invariance() {
    let src1 = br#"
import { Cache } from './cache';

export class TokenManager {
    private cache: Cache;

    constructor() {
        this.cache = new Cache();
    }

    // Refresh the user session with oauth provider
    async refreshSession(userId: string): Promise<string> {
        /* Multiline comment here */
        const token = await this.cache.get(userId);
        return token;
    }
}
"#;

    let src2 = br#"
import { Cache } from './cache';

export class TokenManager {
    private cache: Cache;

    constructor() {
        this.cache = new Cache();
    }

    /**
     * Completely different doc comment!
     * @param userId - user identifier
     */
    async refreshSession(userId: string): Promise<string> {
        // Different line comment
        const token = await this.cache.get(userId);
        return token;
    }
}
"#;

    let symbols1 = extract_symbols(Path::new("src/auth/token_manager.ts"), src1).unwrap();
    let symbols2 = extract_symbols(Path::new("src/auth/token_manager.ts"), src2).unwrap();

    let method1 = symbols1
        .iter()
        .find(|s| s.scope_chain == "TokenManager.refreshSession")
        .expect("Should find TokenManager.refreshSession");
    let method2 = symbols2
        .iter()
        .find(|s| s.scope_chain == "TokenManager.refreshSession")
        .expect("Should find TokenManager.refreshSession in src2");

    assert_eq!(
        method1.uri,
        "src/auth/token_manager.ts#TokenManager.refreshSession"
    );
    assert!(method1.ast_hash.starts_with("ast:sha256:"));
    assert_eq!(method1.ast_hash.len(), 27); // "ast:sha256:" (11) + 16 chars

    // Ast hash MUST be invariant to comments!
    assert_eq!(method1.ast_hash, method2.ast_hash);
}

#[test]
fn test_rust_golden_and_whitespace_invariance() {
    let src_clean = br#"
pub struct ConnectionPool {
    size: usize,
}

impl ConnectionPool {
    pub fn acquire(&self) -> bool {
        self.size > 0
    }
}
"#;

    let src_crlf_and_spaced = b"\r\n\r\npub struct ConnectionPool {\r\n    size: usize,\r\n}\r\n\r\nimpl ConnectionPool {\r\n    // Check available connections\r\n    pub fn acquire(&self) -> bool {\r\n        \r\n        self.size > 0\r\n    }\r\n}\r\n";

    let symbols1 = extract_symbols(Path::new("src/pool/connection.rs"), src_clean).unwrap();
    let symbols2 =
        extract_symbols(Path::new("src/pool/connection.rs"), src_crlf_and_spaced).unwrap();

    let method1 = symbols1
        .iter()
        .find(|s| s.scope_chain == "ConnectionPool::acquire")
        .expect("Should find ConnectionPool::acquire");
    let method2 = symbols2
        .iter()
        .find(|s| s.scope_chain == "ConnectionPool::acquire")
        .expect("Should find ConnectionPool::acquire in crlf version");

    assert_eq!(
        method1.uri,
        "src/pool/connection.rs#ConnectionPool::acquire"
    );
    assert_eq!(method1.ast_hash, method2.ast_hash);
}

#[test]
fn test_python_docstring_invariance() {
    let py1 = br#"
class Worker:
    def process(self, item):
        """Original docstring explaining work."""
        return item * 2
"#;

    let py2 = br#"
class Worker:
    def process(self, item):
        # Changed comments and docstrings
        """Completely new docstring with different text."""
        return item * 2
"#;

    let symbols1 = extract_symbols(Path::new("app/worker.py"), py1).unwrap();
    let symbols2 = extract_symbols(Path::new("app/worker.py"), py2).unwrap();

    let method1 = symbols1
        .iter()
        .find(|s| s.scope_chain == "Worker.process")
        .unwrap();
    let method2 = symbols2
        .iter()
        .find(|s| s.scope_chain == "Worker.process")
        .unwrap();

    assert_eq!(method1.ast_hash, method2.ast_hash);
}

#[test]
fn test_go_receiver_method() {
    let go_src = br#"
package queue

type Worker struct {
    id string
}

func (w *Worker) Dispatch(job string) error {
    // dispatch logic
    return nil
}
"#;

    let symbols = extract_symbols(Path::new("pkg/queue/worker.go"), go_src).unwrap();
    let dispatch = symbols
        .iter()
        .find(|s| s.scope_chain == "Worker.Dispatch")
        .unwrap();

    assert_eq!(dispatch.uri, "pkg/queue/worker.go#Worker.Dispatch");
    assert_eq!(dispatch.kind, SymbolKind::Method);
}

#[test]
fn test_semantic_mutation_changes_hash() {
    let src1 = br#"
function calculateTax(amount: number): number {
    return amount * 0.15;
}
"#;

    let src2 = br#"
function calculateTax(amount: number): number {
    return amount * 0.20;
}
"#;

    let s1 = extract_symbols(Path::new("tax.ts"), src1).unwrap();
    let s2 = extract_symbols(Path::new("tax.ts"), src2).unwrap();

    assert_ne!(s1[0].ast_hash, s2[0].ast_hash);
}

#[test]
fn test_unsupported_file_fallback() {
    let text = b"# Documentation\n\nSome instructions here\n";
    let symbols = extract_symbols(Path::new("docs/README.md"), text).unwrap();

    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].uri, "docs/README.md#<file>");
    assert_eq!(symbols[0].kind, SymbolKind::File);
}
