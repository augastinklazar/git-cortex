use crate::hash::{compute_ast_hash_for_lang, compute_text_fallback_hash};
use crate::lang::Language;
use crate::uri::{format_symbol_uri, normalize_rel_path};
use cortex_core::error::{CortexError, Result};
use std::ops::Range;
use std::path::Path;
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Trait,
    Interface,
    Enum,
    Type,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Symbol {
    pub uri: String,
    pub scope_chain: String,
    pub kind: SymbolKind,
    pub ast_hash: String,
    pub byte_range: Range<usize>,
    pub line_range: Range<u32>,
}

pub fn extract_symbols(rel_path: &Path, source: &[u8]) -> Result<Vec<Symbol>> {
    let norm_path = normalize_rel_path(rel_path);

    // Skip binary files (null bytes) and files > 2 MB
    if source.len() > 2 * 1024 * 1024 || source[..source.len().min(1024)].contains(&0) {
        let hash = cortex_core::jcs::sha256_digest(source);
        return Ok(vec![Symbol {
            uri: format_symbol_uri(&norm_path, "<binary>"),
            scope_chain: "<binary>".to_string(),
            kind: SymbolKind::File,
            ast_hash: format!("ast:sha256:{}", &hash[7..23]),
            byte_range: 0..source.len(),
            line_range: 1..2,
        }]);
    }

    let lang = Language::from_path(rel_path);

    if !lang.is_supported() {
        // Fallback for non-supported files: whole file symbol
        let text = String::from_utf8_lossy(source);
        let hash = compute_text_fallback_hash(&text);
        let line_count = text.lines().count().max(1) as u32;

        return Ok(vec![Symbol {
            uri: format_symbol_uri(&norm_path, "<file>"),
            scope_chain: "<file>".to_string(),
            kind: SymbolKind::File,
            ast_hash: hash,
            byte_range: 0..source.len(),
            line_range: 1..line_count + 1,
        }]);
    }

    let mut parser = Parser::new();
    let ts_lang = match lang {
        Language::Rust => tree_sitter_rust::language(),
        Language::TypeScript => tree_sitter_typescript::language_typescript(),
        Language::Tsx => tree_sitter_typescript::language_tsx(),
        Language::JavaScript => tree_sitter_javascript::language(),
        Language::Python => tree_sitter_python::language(),
        Language::Go => tree_sitter_go::language(),
        Language::Unsupported => unreachable!(),
    };

    parser
        .set_language(&ts_lang)
        .map_err(|e| CortexError::Other(format!("Failed to set tree-sitter language: {:?}", e)))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| CortexError::Other("Tree-sitter failed to parse source".to_string()))?;

    let root = tree.root_node();
    let mut symbols = Vec::new();
    let mut scope_stack = Vec::new();

    walk_and_extract(
        &root,
        source,
        &norm_path,
        lang,
        &mut scope_stack,
        &mut symbols,
    );

    // If no symbols were found in a supported file, emit whole file fallback
    if symbols.is_empty() {
        let text = String::from_utf8_lossy(source);
        let hash = compute_text_fallback_hash(&text);
        let line_count = text.lines().count().max(1) as u32;

        symbols.push(Symbol {
            uri: format_symbol_uri(&norm_path, "<file>"),
            scope_chain: "<file>".to_string(),
            kind: SymbolKind::File,
            ast_hash: hash,
            byte_range: 0..source.len(),
            line_range: 1..line_count + 1,
        });
    }

    Ok(symbols)
}

fn walk_and_extract(
    node: &Node,
    source: &[u8],
    rel_path: &str,
    lang: Language,
    scope_stack: &mut Vec<String>,
    symbols: &mut Vec<Symbol>,
) {
    let mut entered_scope = None;
    let sep = lang.scope_separator();

    match (lang, node.kind()) {
        // --- Rust ---
        (Language::Rust, "mod_item") => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    entered_scope = Some(name.to_string());
                }
            }
        }
        (Language::Rust, "function_item") => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let scope = build_scope(scope_stack, name, sep);
                    let kind = if scope_stack.is_empty() {
                        SymbolKind::Function
                    } else {
                        SymbolKind::Method
                    };
                    record_symbol(node, source, rel_path, scope, kind, symbols, lang);
                }
            }
        }
        (Language::Rust, "struct_item")
        | (Language::Rust, "enum_item")
        | (Language::Rust, "trait_item") => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let scope = build_scope(scope_stack, name, sep);
                    let kind = match node.kind() {
                        "struct_item" => SymbolKind::Struct,
                        "enum_item" => SymbolKind::Enum,
                        _ => SymbolKind::Trait,
                    };
                    record_symbol(node, source, rel_path, scope.clone(), kind, symbols, lang);
                    entered_scope = Some(name.to_string());
                }
            }
        }
        (Language::Rust, "impl_item") => {
            if let Some(type_node) = node.child_by_field_name("type") {
                if let Ok(raw_type) = type_node.utf8_text(source) {
                    let base_type = raw_type.split('<').next().unwrap_or(raw_type).trim();
                    let scope_label = if let Some(trait_node) = node.child_by_field_name("trait") {
                        let raw_trait = trait_node.utf8_text(source).unwrap_or("").trim();
                        let base_trait = raw_trait.split('<').next().unwrap_or(raw_trait).trim();
                        if !base_trait.is_empty() {
                            format!("{}<as {}>", base_type, base_trait)
                        } else {
                            base_type.to_string()
                        }
                    } else {
                        base_type.to_string()
                    };
                    if !scope_label.is_empty() {
                        entered_scope = Some(scope_label);
                    }
                }
            }
        }

        // --- TypeScript / JavaScript ---
        (Language::TypeScript | Language::Tsx | Language::JavaScript, "function_declaration") => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let scope = build_scope(scope_stack, name, sep);
                    record_symbol(
                        node,
                        source,
                        rel_path,
                        scope,
                        SymbolKind::Function,
                        symbols,
                        lang,
                    );
                }
            }
        }
        (Language::TypeScript | Language::Tsx | Language::JavaScript, "method_definition") => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let scope = build_scope(scope_stack, name, sep);
                    record_symbol(
                        node,
                        source,
                        rel_path,
                        scope,
                        SymbolKind::Method,
                        symbols,
                        lang,
                    );
                }
            }
        }
        (Language::TypeScript | Language::Tsx | Language::JavaScript, "class_declaration") => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let scope = build_scope(scope_stack, name, sep);
                    record_symbol(
                        node,
                        source,
                        rel_path,
                        scope.clone(),
                        SymbolKind::Class,
                        symbols,
                        lang,
                    );
                    entered_scope = Some(name.to_string());
                }
            }
        }
        (Language::TypeScript | Language::Tsx, "interface_declaration") => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let scope = build_scope(scope_stack, name, sep);
                    record_symbol(
                        node,
                        source,
                        rel_path,
                        scope.clone(),
                        SymbolKind::Interface,
                        symbols,
                        lang,
                    );
                    entered_scope = Some(name.to_string());
                }
            }
        }
        (Language::TypeScript | Language::Tsx | Language::JavaScript, "variable_declarator") => {
            // Check for arrow function assignment: const foo = () => ...
            if let (Some(name_node), Some(value_node)) = (
                node.child_by_field_name("name"),
                node.child_by_field_name("value"),
            ) {
                if value_node.kind() == "arrow_function" || value_node.kind() == "function" {
                    if let Ok(name) = name_node.utf8_text(source) {
                        let scope = build_scope(scope_stack, name, sep);
                        record_symbol(
                            node,
                            source,
                            rel_path,
                            scope,
                            SymbolKind::Function,
                            symbols,
                            lang,
                        );
                    }
                }
            }
        }

        // --- Python ---
        (Language::Python, "function_definition") => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let scope = build_scope(scope_stack, name, sep);
                    let kind = if scope_stack.is_empty() {
                        SymbolKind::Function
                    } else {
                        SymbolKind::Method
                    };
                    record_symbol(node, source, rel_path, scope, kind, symbols, lang);
                }
            }
        }
        (Language::Python, "class_definition") => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let scope = build_scope(scope_stack, name, sep);
                    record_symbol(
                        node,
                        source,
                        rel_path,
                        scope.clone(),
                        SymbolKind::Class,
                        symbols,
                        lang,
                    );
                    entered_scope = Some(name.to_string());
                }
            }
        }

        // --- Go ---
        (Language::Go, "function_declaration") => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let scope = build_scope(scope_stack, name, sep);
                    record_symbol(
                        node,
                        source,
                        rel_path,
                        scope,
                        SymbolKind::Function,
                        symbols,
                        lang,
                    );
                }
            }
        }
        (Language::Go, "method_declaration") => {
            let receiver_type = extract_go_receiver_type(node, source);
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let scope = if let Some(rec) = receiver_type {
                        format!("{}{}{}", rec, sep, name)
                    } else {
                        build_scope(scope_stack, name, sep)
                    };
                    record_symbol(
                        node,
                        source,
                        rel_path,
                        scope,
                        SymbolKind::Method,
                        symbols,
                        lang,
                    );
                }
            }
        }
        (Language::Go, "type_declaration") => {
            for i in 0..node.named_child_count() {
                if let Some(spec) = node.named_child(i) {
                    if spec.kind() == "type_spec" || spec.kind() == "type_alias" {
                        if let Some(name_node) = spec.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(source) {
                                let scope = build_scope(scope_stack, name, sep);
                                let sym_kind =
                                    if let Some(type_node) = spec.child_by_field_name("type") {
                                        match type_node.kind() {
                                            "interface_type" => SymbolKind::Interface,
                                            "struct_type" => SymbolKind::Struct,
                                            _ => SymbolKind::Type,
                                        }
                                    } else {
                                        SymbolKind::Type
                                    };
                                record_symbol(
                                    &spec, source, rel_path, scope, sym_kind, symbols, lang,
                                );
                            }
                        }
                    }
                }
            }
        }

        _ => {}
    }

    if let Some(ref scope_name) = entered_scope {
        scope_stack.push(scope_name.clone());
    }

    // Recurse children
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            walk_and_extract(&child, source, rel_path, lang, scope_stack, symbols);
        }
    }

    if entered_scope.is_some() {
        scope_stack.pop();
    }
}

fn extract_go_receiver_type(node: &Node, source: &[u8]) -> Option<String> {
    let rec_node = node.child_by_field_name("receiver")?;
    let param = rec_node.named_child(0)?;
    let type_node = param.child_by_field_name("type")?;
    let raw_text = type_node.utf8_text(source).ok()?;
    // Strip pointer star e.g. *ConnectionPool -> ConnectionPool
    Some(raw_text.trim_start_matches('*').trim().to_string())
}

fn build_scope(stack: &[String], name: &str, sep: &str) -> String {
    if stack.is_empty() {
        name.to_string()
    } else {
        format!("{}{}{}", stack.join(sep), sep, name)
    }
}

fn record_symbol(
    node: &Node,
    source: &[u8],
    rel_path: &str,
    scope_chain: String,
    kind: SymbolKind,
    symbols: &mut Vec<Symbol>,
    lang: Language,
) {
    let ast_hash = compute_ast_hash_for_lang(node, source, lang);
    let start_pos = node.start_position();
    let end_pos = node.end_position();

    // Disambiguate duplicate/overloaded symbol scopes with ordinal suffix
    let final_scope = disambiguate_scope(symbols, &scope_chain);

    let end_row = if end_pos.column == 0 && end_pos.row > start_pos.row {
        end_pos.row as u32 + 1
    } else {
        end_pos.row as u32 + 2
    };
    let line_range = (start_pos.row as u32 + 1)..end_row;

    symbols.push(Symbol {
        uri: format_symbol_uri(rel_path, &final_scope),
        scope_chain: final_scope,
        kind,
        ast_hash,
        byte_range: node.byte_range(),
        line_range,
    });
}

fn disambiguate_scope(existing: &[Symbol], scope: &str) -> String {
    let count = existing
        .iter()
        .filter(|s| s.scope_chain == scope || s.scope_chain.starts_with(&format!("{}@", scope)))
        .count();
    if count == 0 {
        scope.to_string()
    } else {
        format!("{}@{}", scope, count + 1)
    }
}
