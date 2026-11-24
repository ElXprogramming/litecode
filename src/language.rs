use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    C,
    Cpp,
    Python,
    JavaScript,
    TypeScript,
    Json,
    Html,
    Xml,
    Css,
    Go,
    Java,
    Shell,
    Lua,
    Toml,
    Yaml,
    Markdown,
    PlainText,
}

impl Language {
    pub fn from_path_and_content(path: Option<&Path>, content: &str) -> Self {
        if let Some(p) = path {
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                match name {
                    "PKGBUILD" | ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" => {
                        return Self::Shell;
                    }
                    "Cargo.toml" | "Cargo.lock" => return Self::Toml,
                    "Makefile" | "GNUmakefile" => return Self::Shell,
                    _ => {}
                }
            }

            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                match ext.to_ascii_lowercase().as_str() {
                    "rs" => return Self::Rust,
                    "c" | "h" => return Self::C,
                    "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => return Self::Cpp,
                    "py" | "pyw" | "pyi" => return Self::Python,
                    "js" | "mjs" | "cjs" | "jsx" => return Self::JavaScript,
                    "ts" | "mts" | "cts" | "tsx" => return Self::TypeScript,
                    "json" | "jsonc" => return Self::Json,
                    "html" | "htm" | "svelte" | "vue" => return Self::Html,
                    "xml" | "svg" | "ui" | "plist" => return Self::Xml,
                    "css" | "scss" | "less" => return Self::Css,
                    "go" => return Self::Go,
                    "java" | "kt" | "cs" => return Self::Java,
                    "sh" | "bash" | "zsh" | "fish" => return Self::Shell,
                    "lua" => return Self::Lua,
                    "toml" | "ini" | "conf" | "cfg" => return Self::Toml,
                    "yaml" | "yml" => return Self::Yaml,
                    "md" | "markdown" => return Self::Markdown,
                    _ => {}
                }
            }
        }

        // Fallback: inspect shebang or obvious leading content for untitled buffers
        let trimmed = content.trim_start();
        if let Some(first_line) = content.lines().next() {
            if first_line.starts_with("#!") {
                if first_line.contains("python") {
                    return Self::Python;
                }
                if first_line.contains("node") || first_line.contains("deno") {
                    return Self::JavaScript;
                }
                if first_line.contains("sh") || first_line.contains("bash") || first_line.contains("zsh") {
                    return Self::Shell;
                }
                if first_line.contains("lua") {
                    return Self::Lua;
                }
            }
        }
        if trimmed.starts_with("<!DOCTYPE html") || trimmed.starts_with("<html") {
            return Self::Html;
        }
        if trimmed.starts_with("<?xml") {
            return Self::Xml;
        }
        if (trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
        {
            if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
                return Self::Json;
            }
        }

        Self::PlainText
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Python => "Python",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Json => "JSON",
            Self::Html => "HTML",
            Self::Xml => "XML",
            Self::Css => "CSS",
            Self::Go => "Go",
            Self::Java => "Java / C#",
            Self::Shell => "Shell / PKGBUILD",
            Self::Lua => "Lua",
            Self::Toml => "TOML / Config",
            Self::Yaml => "YAML",
            Self::Markdown => "Markdown",
            Self::PlainText => "Plain Text",
        }
    }

    pub fn default_indent_size(self) -> usize {
        match self {
            Self::JavaScript
            | Self::TypeScript
            | Self::Json
            | Self::Html
            | Self::Xml
            | Self::Css
            | Self::Yaml
            | Self::Markdown => 2,
            _ => 4,
        }
    }

    pub fn line_comment_prefix(self) -> Option<&'static str> {
        match self {
            Self::Rust
            | Self::C
            | Self::Cpp
            | Self::JavaScript
            | Self::TypeScript
            | Self::Go
            | Self::Java
            | Self::Css => Some("//"),
            Self::Python | Self::Shell | Self::Toml | Self::Yaml => Some("#"),
            Self::Lua => Some("--"),
            Self::Html | Self::Xml | Self::Json | Self::Markdown | Self::PlainText => None,
        }
    }

    /// Short VS Code-style badge text and CSS class for sidebar & tabs
    pub fn badge(self) -> (&'static str, &'static str) {
        match self {
            Self::Rust => ("RS", "badge-rust"),
            Self::C => ("C", "badge-c"),
            Self::Cpp => ("C+", "badge-cpp"),
            Self::Python => ("PY", "badge-py"),
            Self::JavaScript => ("JS", "badge-js"),
            Self::TypeScript => ("TS", "badge-ts"),
            Self::Json => ("{}", "badge-json"),
            Self::Html => ("<>", "badge-html"),
            Self::Xml => ("<>", "badge-xml"),
            Self::Css => ("#", "badge-css"),
            Self::Go => ("GO", "badge-go"),
            Self::Java => ("JV", "badge-java"),
            Self::Shell => ("$", "badge-sh"),
            Self::Lua => ("LU", "badge-lua"),
            Self::Toml => ("TM", "badge-toml"),
            Self::Yaml => ("YM", "badge-yaml"),
            Self::Markdown => ("MD", "badge-md"),
            Self::PlainText => ("≡", "badge-txt"),
        }
    }

    pub fn all_selectable() -> &'static [Language] {
        &[
            Self::PlainText,
            Self::Rust,
            Self::C,
            Self::Cpp,
            Self::Python,
            Self::JavaScript,
            Self::TypeScript,
            Self::Json,
            Self::Html,
            Self::Xml,
            Self::Css,
            Self::Go,
            Self::Java,
            Self::Shell,
            Self::Lua,
            Self::Toml,
            Self::Yaml,
            Self::Markdown,
        ]
    }
}
