use crate::lang::Language;
use sha2::{Digest, Sha256};
use tree_sitter::Node;

/// Hashes an AST node subtree in a normalized, comment-insensitive manner for a specific language
pub fn compute_ast_hash_for_lang(node: &Node, source: &[u8], lang: Language) -> String {
    let mut hasher = Sha256::new();
    walk_node(node, source, &mut hasher, true, lang);
    let digest = hasher.finalize();
    let hex_full = hex::encode(digest);
    // 64-bit truncated SHA-256 digest = 16 hex chars
    format!("ast:sha256:{}", &hex_full[..16])
}

/// Hashes an AST node subtree with default language settings
pub fn compute_ast_hash(node: &Node, source: &[u8]) -> String {
    compute_ast_hash_for_lang(node, source, Language::Unsupported)
}

/// Fallback hash for whole text files when tree-sitter grammar is not available
pub fn compute_text_fallback_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    // Normalize lines by trimming and ignoring blank lines
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            hasher.update(trimmed.as_bytes());
            hasher.update(b"\n");
        }
    }
    let digest = hex::encode(hasher.finalize());
    format!("ast:sha256:{}", &digest[..16])
}

fn is_comment_kind(kind: &str) -> bool {
    matches!(
        kind,
        "comment"
            | "line_comment"
            | "block_comment"
            | "doc_comment"
            | "inner_doc_comment"
            | "outer_doc_comment"
    )
}

/// Only treat an expression_statement as a Python docstring if it is the very first
/// non-comment statement inside a Python module, class_definition, or function_definition block.
fn is_python_docstring(node: &Node, lang: Language) -> bool {
    if lang != Language::Python || node.kind() != "expression_statement" {
        return false;
    }
    let Some(first_child) = node.named_child(0) else {
        return false;
    };
    if first_child.kind() != "string" && first_child.kind() != "concatenated_string" {
        return false;
    }
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != "block" && parent.kind() != "module" {
        return false;
    }
    for i in 0..parent.named_child_count() {
        if let Some(sibling) = parent.named_child(i) {
            if is_comment_kind(sibling.kind()) {
                continue;
            }
            return sibling.id() == node.id();
        }
    }
    false
}

fn walk_node(node: &Node, source: &[u8], hasher: &mut Sha256, is_root: bool, lang: Language) {
    let kind = node.kind();

    // Skip all comment nodes
    if is_comment_kind(kind) {
        return;
    }

    // Skip python docstrings
    if !is_root && is_python_docstring(node, lang) {
        return;
    }

    let total_children = node.child_count();
    if total_children == 0 {
        // Leaf node: extract token content
        if let Ok(text) = node.utf8_text(source) {
            // Normalize internal CRLF to LF so cross-platform checkouts hash identically
            let normalized = text.trim().replace("\r\n", "\n");
            if !normalized.is_empty() {
                if node.is_named() {
                    hasher.update(kind.as_bytes());
                    hasher.update(b":");
                } else {
                    hasher.update(b"tok:");
                }
                hasher.update(normalized.len().to_string().as_bytes());
                hasher.update(b":");
                hasher.update(normalized.as_bytes());
                hasher.update(b";");
            }
        }
    } else {
        hasher.update(b"(");
        hasher.update(kind.as_bytes());
        hasher.update(b":");
        for i in 0..total_children {
            if let Some(child) = node.child(i) {
                walk_node(&child, source, hasher, false, lang);
            }
        }
        hasher.update(b")");
    }
}
