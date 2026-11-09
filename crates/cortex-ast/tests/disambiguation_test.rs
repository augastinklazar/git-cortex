use cortex_ast::extract_symbols;
use std::path::Path;

#[test]
fn test_disambiguate_multiple_go_init_functions() {
    let go_src = br#"package main

func init() {
    println("first init")
}

func init() {
    println("second init")
}
"#;
    let symbols = extract_symbols(Path::new("pkg/main.go"), go_src).unwrap();
    assert_eq!(symbols.len(), 2);
    assert_eq!(symbols[0].scope_chain, "init");
    assert_eq!(symbols[0].uri, "pkg/main.go#init");

    assert_eq!(symbols[1].scope_chain, "init[2]");
    assert_eq!(symbols[1].uri, "pkg/main.go#init[2]");
}

#[test]
fn test_disambiguate_split_rust_impl_blocks() {
    let rs_src = br#"pub struct Connection;

impl Connection {
    pub fn connect(&self) -> bool { true }
}

#[cfg(test)]
impl Connection {
    pub fn mock_connect(&self) -> bool { false }
}
"#;
    let symbols = extract_symbols(Path::new("src/conn.rs"), rs_src).unwrap();
    let scopes: Vec<&str> = symbols.iter().map(|s| s.scope_chain.as_str()).collect();

    assert!(scopes.contains(&"Connection"));
    assert!(scopes.contains(&"Connection::connect"));
    assert!(scopes.contains(&"Connection::mock_connect"));
}

#[test]
fn test_disambiguate_typescript_function_overloads() {
    let ts_src = br#"function parse(input: string): string;
function parse(input: number): number;
function parse(input: any): any {
    return input;
}
"#;
    let symbols = extract_symbols(Path::new("src/parser.ts"), ts_src).unwrap();
    assert_eq!(symbols.len(), 3);
    assert_eq!(symbols[0].scope_chain, "parse");
    assert_eq!(symbols[1].scope_chain, "parse[2]");
    assert_eq!(symbols[2].scope_chain, "parse[3]");
}

#[test]
fn test_disambiguate_python_functions_same_name() {
    let py_src = br#"def handle():
    pass

def handle():
    pass
"#;
    let symbols = extract_symbols(Path::new("app/handler.py"), py_src).unwrap();
    assert_eq!(symbols.len(), 2);
    assert_eq!(symbols[0].scope_chain, "handle");
    assert_eq!(symbols[1].scope_chain, "handle[2]");
}
