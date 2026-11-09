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
    assert_eq!(method1.ast_hash, "ast:sha256:c598f2edad555974");
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
    assert_eq!(method1.ast_hash, "ast:sha256:b8b19958c0db01b6");
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
    assert_eq!(method1.ast_hash, "ast:sha256:3ddbef0a5ba9c4ef");
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
    assert_eq!(dispatch.ast_hash, "ast:sha256:cac2b372440dc329");
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

#[test]
fn test_operator_sensitivity_in_ast_hash() {
    let src_add = b"fn calc(a: i32, b: i32) -> i32 { a + b }";
    let src_sub = b"fn calc(a: i32, b: i32) -> i32 { a - b }";
    let src_eq = b"fn check(a: i32, b: i32) -> bool { a == b }";
    let src_ne = b"fn check(a: i32, b: i32) -> bool { a != b }";

    let sym_add = extract_symbols(Path::new("math.rs"), src_add).unwrap();
    let sym_sub = extract_symbols(Path::new("math.rs"), src_sub).unwrap();
    let sym_eq = extract_symbols(Path::new("math.rs"), src_eq).unwrap();
    let sym_ne = extract_symbols(Path::new("math.rs"), src_ne).unwrap();

    assert_ne!(
        sym_add[0].ast_hash, sym_sub[0].ast_hash,
        "Operators + and - must produce distinct AST hashes"
    );
    assert_ne!(
        sym_eq[0].ast_hash, sym_ne[0].ast_hash,
        "Operators == and != must produce distinct AST hashes"
    );
}

#[test]
fn test_binary_fallback_hash_format() {
    let binary_data = vec![0u8; 128];
    let syms = extract_symbols(Path::new("data.bin"), &binary_data).unwrap();
    assert_eq!(syms.len(), 1);
    assert_eq!(syms[0].uri, "data.bin#<binary>");
    assert!(syms[0].ast_hash.starts_with("ast:sha256:"));
    assert_eq!(syms[0].ast_hash.len(), 27);
    assert!(!syms[0].ast_hash.contains("sha256:sha256:"));
}

#[test]
fn test_rust_trait_impl_scope_disambiguation() {
    let src = br#"
struct MyType;

impl From<String> for MyType {
    fn from(_: String) -> Self { MyType }
}

impl From<i32> for MyType {
    fn from(_: i32) -> Self { MyType }
}
"#;
    let syms = extract_symbols(Path::new("src/my_type.rs"), src).unwrap();
    let scopes: Vec<&str> = syms.iter().map(|s| s.scope_chain.as_str()).collect();
    assert!(
        scopes.contains(&"MyType<as From>::from") || scopes.contains(&"MyType<as From>::from@2")
    );
}

#[test]
fn test_python_decorator_and_async_generator() {
    let py_src = br#"
@retry(attempts=3)
@instrument(name="event_stream")
async def stream_events(topic: str, batch_size: int = 100):
    """Docstring explaining async event stream generation."""
    async for message in fetch_stream(topic):
        yield transform(message, batch_size)
"#;
    let syms = extract_symbols(Path::new("services/stream.py"), py_src).unwrap();
    assert_eq!(syms.len(), 1);
    let func = &syms[0];
    assert_eq!(func.scope_chain, "stream_events");
    assert_eq!(func.kind, SymbolKind::Function);
    assert_eq!(func.ast_hash, "ast:sha256:3dea939e0746a15d");
}

#[test]
fn test_typescript_namespace_and_interface() {
    let ts_src = br#"
export namespace Security {
    export interface AuthConfig {
        tokenExpirySecs: number;
    }

    export class KeyStore {
        async fetchKey(keyId: string): Promise<string> {
            return keyId;
        }
    }
}
"#;
    let syms = extract_symbols(Path::new("src/security/vault.ts"), ts_src).unwrap();
    assert_eq!(syms.len(), 3);

    let iface = syms.iter().find(|s| s.scope_chain == "AuthConfig").unwrap();
    assert_eq!(iface.kind, SymbolKind::Interface);
    assert_eq!(iface.ast_hash, "ast:sha256:4014a2375fe9870f");

    let cls = syms.iter().find(|s| s.scope_chain == "KeyStore").unwrap();
    assert_eq!(cls.kind, SymbolKind::Class);
    assert_eq!(cls.ast_hash, "ast:sha256:2745cb6623b6cf69");

    let meth = syms
        .iter()
        .find(|s| s.scope_chain == "KeyStore.fetchKey")
        .unwrap();
    assert_eq!(meth.kind, SymbolKind::Method);
    assert_eq!(meth.ast_hash, "ast:sha256:9ae7ad335ac3ea4e");
}

#[test]
fn test_rust_trait_impl_associated_types() {
    let rs_src = br#"
pub trait Transformer<T> {
    type Output;
    fn transform(&self, input: T) -> Self::Output;
}

pub struct Upper;

impl Transformer<&str> for Upper {
    type Output = String;
    fn transform(&self, input: &str) -> Self::Output {
        input.to_uppercase()
    }
}
"#;
    let syms = extract_symbols(Path::new("src/transform.rs"), rs_src).unwrap();
    assert_eq!(syms.len(), 3);

    let trt = syms
        .iter()
        .find(|s| s.scope_chain == "Transformer")
        .unwrap();
    assert_eq!(trt.kind, SymbolKind::Trait);
    assert_eq!(trt.ast_hash, "ast:sha256:e4fc1bd3cb25f710");

    let st = syms.iter().find(|s| s.scope_chain == "Upper").unwrap();
    assert_eq!(st.kind, SymbolKind::Struct);
    assert_eq!(st.ast_hash, "ast:sha256:8c07bf1974d88d36");

    let mth = syms
        .iter()
        .find(|s| s.scope_chain == "Upper<as Transformer>::transform")
        .unwrap();
    assert_eq!(mth.kind, SymbolKind::Method);
    assert_eq!(mth.ast_hash, "ast:sha256:e09169e88fe8e43b");
}
