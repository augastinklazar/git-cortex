use sha2::{Digest, Sha256};
use tree_sitter::Node;

/// Hashes an AST node subtree in a normalized, comment-insensitive manner
pub fn compute_ast_hash(node: &Node, source: &[u8]) -> String {
    let mut hasher = Sha256::new();
    walk_node(node, source, &mut hasher);
    let digest = hex::encode(hasher.finalize());
    // 64-bit truncated SHA-256 digest = 16 hex chars
    format!("ast:sha256:{}", &digest[..16])
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

fn is_comment_node(kind: &str) -> bool {
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

fn is_python_docstring(node: &Node) -> bool {
    if node.kind() == "expression_statement" {
        if let Some(child) = node.named_child(0) {
            if child.kind() == "string" {
                return true;
            }
        }
    }
    false
}

fn walk_node(node: &Node, source: &[u8], hasher: &mut Sha256) {
    let kind = node.kind();

    // Skip all comment nodes
    if is_comment_node(kind) {
        return;
    }

    // Skip python docstrings
    if is_python_docstring(node) {
        return;
    }

    // Hash the syntactic node kind
    hasher.update(kind.as_bytes());
    hasher.update(b":");

    let child_count = node.named_child_count();
    if child_count == 0 {
        // Leaf node: extract token content
        if let Ok(text) = node.utf8_text(source) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                hasher.update(trimmed.as_bytes());
            }
        }
    } else {
        // Walk named children
        for i in 0..child_count {
            if let Some(child) = node.named_child(i) {
                walk_node(&child, source, hasher);
            }
        }
    }
}
