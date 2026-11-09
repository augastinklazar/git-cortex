use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    Unsupported,
}

impl Language {
    pub fn from_path(path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match ext.as_str() {
            "rs" => Self::Rust,
            "ts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "py" => Self::Python,
            "go" => Self::Go,
            _ => Self::Unsupported,
        }
    }

    pub fn is_supported(&self) -> bool {
        !matches!(self, Self::Unsupported)
    }

    pub fn scope_separator(&self) -> &'static str {
        match self {
            Self::Rust => "::",
            _ => ".",
        }
    }
}
