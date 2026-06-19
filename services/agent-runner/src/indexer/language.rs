//! Language detection by file extension. Intentionally minimal: we only parse languages for which
//! we ship a tree-sitter grammar; everything else falls through to windowed chunking.

/// Detect the language of a file from its extension. Returns `None` for unknown/binary files.
pub fn from_path(path: &std::path::Path) -> Option<&'static str> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some("rust"),
        Some("ts" | "tsx") => Some("typescript"),
        Some("js" | "jsx" | "mjs" | "cjs") => Some("javascript"),
        Some("py") => Some("python"),
        Some("go") => Some("go"),
        Some("java") => Some("java"),
        Some("c" | "h") => Some("c"),
        Some("cpp" | "cc" | "cxx" | "hpp") => Some("cpp"),
        Some("md" | "txt" | "toml" | "yaml" | "yml" | "json") => Some("text"),
        _ => None,
    }
}

/// True for languages we have a tree-sitter grammar for (structured chunking available).
pub fn has_grammar(language: &str) -> bool {
    matches!(language, "rust" | "typescript" | "javascript" | "python")
}
