use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::language::Language;

#[derive(Debug, Clone, Copy)]
pub struct FormatOptions {
    pub indent_size: usize,
    pub use_system_formatters: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            indent_size: 4,
            use_system_formatters: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FormatOutcome {
    pub formatted: String,
    pub changed: bool,
    pub engine_used: String,
}

pub fn format_code(
    source: &str,
    lang: Language,
    file_path: Option<&Path>,
    opts: FormatOptions,
) -> FormatOutcome {
    if source.is_empty() {
        return FormatOutcome {
            formatted: String::new(),
            changed: false,
            engine_used: "None (empty)".to_string(),
        };
    }

    let effective_lang = if lang == Language::PlainText {
        guess_code_flavor(source)
    } else {
        lang
    };

    // 1. Try external system formatter if enabled & available on Arch Linux
    if opts.use_system_formatters {
        if let Some((formatted, tool_name)) =
            try_system_formatter(source, effective_lang, file_path, opts.indent_size)
        {
            let changed = formatted != source;
            return FormatOutcome {
                formatted,
                changed,
                engine_used: format!("System ({tool_name})"),
            };
        }
    }

    // 2. Built-in Smart Formatter / Indenter
    let (formatted, engine_label) = match effective_lang {
        Language::Json => format_json_or_braces(source, opts.indent_size),
        Language::Python => (
            format_python_indentation(source, opts.indent_size),
            "Built-in Python Indenter",
        ),
        Language::Html | Language::Xml => (
            format_markup_indentation(source, opts.indent_size, effective_lang == Language::Html),
            "Built-in HTML/XML Indenter",
        ),
        Language::Shell => (
            format_shell_indentation(source, opts.indent_size),
            "Built-in Shell Indenter",
        ),
        Language::Lua => (
            format_lua_indentation(source, opts.indent_size),
            "Built-in Lua Indenter",
        ),
        Language::Yaml | Language::Markdown => (
            normalize_whitespace_and_tabs(source, opts.indent_size),
            "Built-in Whitespace Normalizer",
        ),
        _ => (
            format_brace_indentation(source, opts.indent_size),
            "Built-in Smart Indenter",
        ),
    };

    let changed = formatted != source;
    FormatOutcome {
        formatted,
        changed,
        engine_used: engine_label.to_string(),
    }
}

fn guess_code_flavor(source: &str) -> Language {
    let trimmed = source.trim_start();
    if trimmed.starts_with('<') && trimmed.contains('>') {
        if trimmed.to_ascii_lowercase().contains("<html")
            || trimmed.to_ascii_lowercase().contains("<!doctype html")
            || trimmed.contains("<div")
        {
            return Language::Html;
        }
        return Language::Xml;
    }
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
    {
        return Language::Json;
    }
    for line in source.lines().take(25) {
        let t = line.trim();
        if t.starts_with("def ") && t.ends_with(':')
            || t.starts_with("elif ") && t.ends_with(':')
            || t == "try:"
            || t == "else:" && !source.contains('{')
        {
            return Language::Python;
        }
        if t.starts_with("fn ") && source.contains('{')
            || t.starts_with("use std::")
            || t.contains("println!(")
        {
            return Language::Rust;
        }
        if t.starts_with("#include ") {
            return Language::Cpp;
        }
        if t.starts_with("package ") && source.contains("func ") {
            return Language::Go;
        }
    }
    Language::PlainText
}

fn try_system_formatter(
    source: &str,
    lang: Language,
    file_path: Option<&Path>,
    indent_size: usize,
) -> Option<(String, &'static str)> {
    let indent_str = indent_size.to_string();
    let clang_style = format!("--style={{IndentWidth: {indent_size}}}");
    let filename = file_path
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());

    match lang {
        Language::Rust => run_pipe_cmd(
            "rustfmt",
            &[
                "--edition",
                "2021",
                "--config",
                &format!("tab_spaces={indent_size}"),
            ],
            source,
        )
        .map(|s| (s, "rustfmt")),

        Language::Go => run_pipe_cmd("gofmt", &[], source).map(|s| (s, "gofmt")),

        Language::C | Language::Cpp | Language::Java => {
            let fallback_name = match lang {
                Language::C => "main.c",
                Language::Cpp => "main.cpp",
                _ => "Main.java",
            };
            let assumed = format!("--assume-filename={}", filename.unwrap_or(fallback_name));
            run_pipe_cmd("clang-format", &[&clang_style, &assumed], source)
                .map(|s| (s, "clang-format"))
        }

        Language::Python => {
            let line_len = "--line-length=88";
            if let Some(out) = run_pipe_cmd(
                "ruff",
                &["format", "--stdin-filename", filename.unwrap_or("script.py"), "-"],
                source,
            ) {
                return Some((out, "ruff"));
            }
            if let Some(out) = run_pipe_cmd("black", &["-q", line_len, "-"], source) {
                return Some((out, "black"));
            }
            run_pipe_cmd("autopep8", &["-"], source).map(|s| (s, "autopep8"))
        }

        Language::JavaScript
        | Language::TypeScript
        | Language::Css
        | Language::Html
        | Language::Yaml
        | Language::Markdown => {
            let default_file = match lang {
                Language::JavaScript => "index.js",
                Language::TypeScript => "index.ts",
                Language::Css => "style.css",
                Language::Html => "index.html",
                Language::Yaml => "config.yaml",
                Language::Markdown => "README.md",
                _ => "file.txt",
            };
            let tab_width_arg = format!("--tab-width={indent_size}");
            run_pipe_cmd(
                "prettier",
                &[
                    "--stdin-filepath",
                    filename.unwrap_or(default_file),
                    &tab_width_arg,
                ],
                source,
            )
            .map(|s| (s, "prettier"))
        }

        Language::Shell => {
            run_pipe_cmd("shfmt", &["-i", &indent_str], source).map(|s| (s, "shfmt"))
        }

        Language::Lua => run_pipe_cmd(
            "stylua",
            &[
                "--indent-type",
                "Spaces",
                "--indent-width",
                &indent_str,
                "-",
            ],
            source,
        )
        .map(|s| (s, "stylua")),

        Language::Toml => run_pipe_cmd("taplo", &["fmt", "-"], source).map(|s| (s, "taplo")),

        Language::Xml => {
            // xmllint respects XMLLINT_INDENT
            run_pipe_cmd_with_env(
                "xmllint",
                &["--format", "-"],
                &[("XMLLINT_INDENT", &" ".repeat(indent_size))],
                source,
            )
            .map(|s| (s, "xmllint"))
        }

        Language::Json | Language::PlainText => None,
    }
}

fn run_pipe_cmd(program: &str, args: &[&str], input: &str) -> Option<String> {
    run_pipe_cmd_with_env(program, args, &[], input)
}

fn run_pipe_cmd_with_env(
    program: &str,
    args: &[&str],
    envs: &[(&str, &str)],
    input: &str,
) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in envs {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(input.as_bytes()).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    }

    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8(output.stdout).ok()?;
    if text.trim().is_empty() && !input.trim().is_empty() {
        return None;
    }
    Some(text)
}

fn format_json_or_braces(source: &str, indent_size: usize) -> (String, &'static str) {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(source) {
        if let Ok(pretty_two) = serde_json::to_string_pretty(&val) {
            let mut out = if indent_size == 2 {
                pretty_two
            } else {
                let mut custom = String::with_capacity(pretty_two.len() + 64);
                for (idx, line) in pretty_two.lines().enumerate() {
                    if idx > 0 {
                        custom.push('\n');
                    }
                    let leading_spaces = line.bytes().take_while(|&b| b == b' ').count();
                    let depth = leading_spaces / 2;
                    custom.push_str(&" ".repeat(depth * indent_size));
                    custom.push_str(&line[leading_spaces..]);
                }
                custom
            };
            out.push('\n');
            return (out, "Built-in JSON Formatter");
        }
    }
    (
        format_brace_indentation(source, indent_size),
        "Built-in Smart Indenter",
    )
}

/// Lexer-aware smart indentation formatter for C, C++, Rust, JS/TS, Java, Go, CSS, JSONC, etc.
pub fn format_brace_indentation(source: &str, indent_size: usize) -> String {
    let mut out = String::with_capacity(source.len() + 128);
    let mut indent_level: i32 = 0;
    let mut in_block_comment = false;
    let mut in_backtick_string = false;
    let mut consecutive_blank_lines = 0usize;
    // Track switch-case extra indent stacks: stores the brace depth of `switch` blocks
    let mut case_active_at_depth: Vec<i32> = Vec::new();

    for raw_line in source.lines() {
        // Inside multi-line template string: preserve verbatim
        if in_backtick_string {
            out.push_str(raw_line);
            out.push('\n');
            scan_backtick_line_end(raw_line, &mut in_backtick_string);
            continue;
        }

        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            consecutive_blank_lines += 1;
            if consecutive_blank_lines <= 2 {
                out.push('\n');
            }
            continue;
        }
        consecutive_blank_lines = 0;

        // Continuing a multi-line /* ... */ comment
        if in_block_comment {
            let base_spaces = " ".repeat((indent_level.max(0) as usize) * indent_size);
            if trimmed.starts_with('*') {
                out.push_str(&base_spaces);
                out.push(' ');
                out.push_str(trimmed);
            } else {
                out.push_str(&base_spaces);
                out.push_str(trimmed);
            }
            out.push('\n');

            if let Some(end_idx) = trimmed.find("*/") {
                in_block_comment = false;
                let rest = &trimmed[end_idx + 2..];
                let stats = scan_code_line(
                    rest,
                    &mut in_block_comment,
                    &mut in_backtick_string,
                );
                indent_level = (indent_level + stats.net_delta).max(0);
            }
            continue;
        }

        // Preprocessor directives in C/C++ (#include, #define, #ifdef, #endif)
        if is_c_preprocessor(trimmed) {
            out.push_str(trimmed);
            out.push('\n');
            continue;
        }

        let normalized_line = tidy_brace_spacing(trimmed);
        let stats = scan_code_line(
            &normalized_line,
            &mut in_block_comment,
            &mut in_backtick_string,
        );

        // Pop any case indent contexts if closing braces exit their switch block
        let effective_before_close = (indent_level - stats.leading_closers).max(0);
        while let Some(&switch_depth) = case_active_at_depth.last() {
            if effective_before_close < switch_depth {
                case_active_at_depth.pop();
            } else {
                break;
            }
        }

        let is_case_label = is_switch_case_or_default(&normalized_line);
        if is_case_label {
            // If we aren't yet tracking a case at this switch depth, register it
            if case_active_at_depth.last().copied() != Some(indent_level) {
                case_active_at_depth.push(indent_level);
            }
        }

        let case_bonus = if is_case_label {
            // The `case X:` label itself sits 1 level shallower than its statements
            case_active_at_depth.len().saturating_sub(1) as i32
        } else {
            case_active_at_depth.len() as i32
        };

        let line_indent = (effective_before_close + case_bonus).max(0) as usize;
        out.push_str(&" ".repeat(line_indent * indent_size));
        out.push_str(&normalized_line);
        out.push('\n');

        indent_level = (indent_level + stats.net_delta).max(0);
        while let Some(&switch_depth) = case_active_at_depth.last() {
            if indent_level < switch_depth {
                case_active_at_depth.pop();
            } else {
                break;
            }
        }
    }

    preserve_trailing_newline_preference(source, out)
}

struct LineBracketStats {
    leading_closers: i32,
    net_delta: i32,
}

fn scan_code_line(
    line: &str,
    in_block_comment: &mut bool,
    in_backtick_string: &mut bool,
) -> LineBracketStats {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    let mut leading_closers = 0i32;
    let mut still_at_start = true;
    let mut net_delta = 0i32;

    while i < len {
        let b = bytes[i];

        if *in_block_comment {
            if b == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                *in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comment //
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            break;
        }

        // Block comment /*
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            *in_block_comment = true;
            i += 2;
            continue;
        }

        // Strings: "...", '...', `...`
        if b == b'"' || b == b'\'' {
            still_at_start = false;
            let quote = b;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        if b == b'`' {
            still_at_start = false;
            *in_backtick_string = true;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'`' {
                    *in_backtick_string = false;
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        match b {
            b'{' | b'[' | b'(' => {
                still_at_start = false;
                net_delta += 1;
            }
            b'}' | b']' | b')' => {
                if still_at_start {
                    leading_closers += 1;
                }
                net_delta -= 1;
            }
            _ => {
                still_at_start = false;
            }
        }

        i += 1;
    }

    LineBracketStats {
        leading_closers,
        net_delta,
    }
}

fn scan_backtick_line_end(line: &str, in_backtick_string: &mut bool) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            *in_backtick_string = !*in_backtick_string;
        }
        i += 1;
    }
}

fn is_c_preprocessor(trimmed: &str) -> bool {
    if !trimmed.starts_with('#') || trimmed.starts_with("#[") || trimmed.starts_with("#![") {
        return false;
    }
    let rest = trimmed[1..].trim_start();
    const DIRECTIVES: &[&str] = &[
        "include", "define", "undef", "ifdef", "ifndef", "if ", "elif ", "else", "endif", "pragma",
        "error",
    ];
    DIRECTIVES.iter().any(|d| rest.starts_with(d))
}

fn is_switch_case_or_default(trimmed: &str) -> bool {
    if let Some(rest) = trimmed.strip_prefix("case ") {
        return rest.contains(':') && !rest.contains("=>");
    }
    trimmed == "default:" || trimmed.starts_with("default:")
}

/// Gently normalizes squished `}else{` or `){` outside strings/comments without altering literals
fn tidy_brace_spacing(trimmed: &str) -> String {
    let mut res = trimmed.to_string();
    if !res.contains('"') && !res.contains('\'') && !res.contains("//") {
        res = res
            .replace("}else{", "} else {")
            .replace("}else ", "} else ")
            .replace(" else{", " else {")
            .replace("}catch(", "} catch (")
            .replace("}catch (", "} catch (")
            .replace("}finally{", "} finally {");
    }
    res
}

/// Smart Python indentation formatter:
/// - Converts tabs/irregular spaces into clean multiples of `indent_size`
/// - Enforces `+1` indent after headers ending in `:` (e.g. `def`, `if`, `for`, `class`, `try`, `with`)
/// - Aligns `elif`, `else:`, `except`, `finally:` with their matching block header
/// - Indents multi-line `()`, `[]`, `{}` expressions cleanly
#[derive(Clone, Copy, PartialEq, Eq)]
enum PyHeaderKind {
    If,
    Try,
    Loop,
    Other,
}

pub fn format_python_indentation(source: &str, indent_size: usize) -> String {
    #[derive(Clone, Copy)]
    struct PyBlock {
        logical_level: usize,
        raw_cols: usize,
        kind: PyHeaderKind,
    }

    let mut out = String::with_capacity(source.len() + 64);
    let mut blocks: Vec<PyBlock> = Vec::new();
    let mut logical_level: usize = 0;
    let mut current_raw_cols: usize = 0;
    let mut prev_header_kind: Option<PyHeaderKind> = None;
    let mut open_paren_depth: i32 = 0;
    let mut consecutive_blank_lines = 0usize;

    for raw_line in source.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            consecutive_blank_lines += 1;
            if consecutive_blank_lines <= 2 {
                out.push('\n');
            }
            continue;
        }
        consecutive_blank_lines = 0;

        let raw_cols = visual_indent_columns(raw_line, indent_size);

        if open_paren_depth > 0 {
            let leading_close = python_leading_closers(trimmed);
            let effective_paren = (open_paren_depth - leading_close).max(0) as usize;
            let total_level = logical_level + effective_paren;
            out.push_str(&" ".repeat(total_level * indent_size));
            out.push_str(trimmed);
            out.push('\n');

            let (delta, ends_colon) = scan_python_line(trimmed);
            open_paren_depth = (open_paren_depth + delta).max(0);
            if open_paren_depth == 0 {
                prev_header_kind = if ends_colon {
                    Some(classify_py_header(trimmed))
                } else {
                    None
                };
            }
            continue;
        }

        let companion = classify_py_companion(trimmed);

        if let Some(kind) = prev_header_kind.take() {
            blocks.push(PyBlock {
                logical_level,
                raw_cols: current_raw_cols,
                kind,
            });
            logical_level += 1;
            current_raw_cols = raw_cols;
        } else if let Some(comp_target) = companion {
            // Match against the nearest compatible block header
            if let Some(pos) = blocks.iter().rposition(|b| {
                let kind_ok = match comp_target {
                    PyCompanion::Elif => b.kind == PyHeaderKind::If,
                    PyCompanion::Else => {
                        matches!(b.kind, PyHeaderKind::If | PyHeaderKind::Loop | PyHeaderKind::Try)
                    }
                    PyCompanion::ExceptOrFinally => b.kind == PyHeaderKind::Try,
                };
                kind_ok && (raw_cols == b.raw_cols || raw_cols == 0)
            }) {
                logical_level = blocks[pos].logical_level;
                current_raw_cols = blocks[pos].raw_cols;
                // Pop inner blocks above this matched header; keep or replace header on colon below
                blocks.truncate(pos);
            } else {
                while let Some(top) = blocks.last() {
                    if raw_cols <= top.raw_cols {
                        logical_level = top.logical_level;
                        blocks.pop();
                    } else {
                        break;
                    }
                }
                current_raw_cols = raw_cols;
            }
        } else if raw_cols > current_raw_cols {
            blocks.push(PyBlock {
                logical_level,
                raw_cols: current_raw_cols,
                kind: PyHeaderKind::Other,
            });
            logical_level += 1;
            current_raw_cols = raw_cols;
        } else if raw_cols < current_raw_cols {
            while let Some(top) = blocks.last() {
                if raw_cols <= top.raw_cols {
                    logical_level = top.logical_level;
                    current_raw_cols = top.raw_cols;
                    blocks.pop();
                    if raw_cols == current_raw_cols {
                        break;
                    }
                } else {
                    break;
                }
            }
            current_raw_cols = raw_cols;
        }

        out.push_str(&" ".repeat(logical_level * indent_size));
        out.push_str(trimmed);
        out.push('\n');

        let (delta, ends_colon) = scan_python_line(trimmed);
        open_paren_depth = (open_paren_depth + delta).max(0);
        prev_header_kind = if open_paren_depth == 0 && ends_colon {
            Some(classify_py_header(trimmed))
        } else {
            None
        };
    }

    preserve_trailing_newline_preference(source, out)
}

#[derive(Clone, Copy)]
enum PyCompanion {
    Elif,
    Else,
    ExceptOrFinally,
}

fn classify_py_companion(trimmed: &str) -> Option<PyCompanion> {
    if trimmed.starts_with("elif ") && trimmed.contains(':') {
        Some(PyCompanion::Elif)
    } else if trimmed == "else:" || trimmed.starts_with("else:") {
        Some(PyCompanion::Else)
    } else if trimmed == "except:"
        || (trimmed.starts_with("except ") && trimmed.contains(':'))
        || trimmed == "finally:"
        || trimmed.starts_with("finally:")
    {
        Some(PyCompanion::ExceptOrFinally)
    } else {
        None
    }
}

fn classify_py_header(trimmed: &str) -> PyHeaderKind {
    if trimmed.starts_with("if ") || trimmed.starts_with("elif ") {
        PyHeaderKind::If
    } else if trimmed == "try:" || trimmed.starts_with("try:") || trimmed.starts_with("except") {
        PyHeaderKind::Try
    } else if trimmed.starts_with("for ")
        || trimmed.starts_with("while ")
        || trimmed.starts_with("async for ")
    {
        PyHeaderKind::Loop
    } else {
        PyHeaderKind::Other
    }
}

fn visual_indent_columns(line: &str, tab_width: usize) -> usize {
    let mut cols = 0;
    for ch in line.chars() {
        match ch {
            ' ' => cols += 1,
            '\t' => cols += tab_width - (cols % tab_width),
            _ => break,
        }
    }
    cols
}

fn python_leading_closers(trimmed: &str) -> i32 {
    let mut count = 0;
    for b in trimmed.bytes() {
        match b {
            b')' | b']' | b'}' => count += 1,
            b' ' | b'\t' => {}
            _ => break,
        }
    }
    count
}

fn scan_python_line(trimmed: &str) -> (i32, bool) {
    let bytes = trimmed.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut delta = 0i32;
    let mut last_code_byte: Option<u8> = None;

    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'#' {
            break;
        }
        if b == b'"' || b == b'\'' {
            let q = b;
            // Check triple quote
            if i + 2 < len && bytes[i + 1] == q && bytes[i + 2] == q {
                i += 3;
                while i + 2 < len {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == q && bytes[i + 1] == q && bytes[i + 2] == q {
                        i += 3;
                        break;
                    }
                    i += 1;
                }
            } else {
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == q {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            last_code_byte = Some(q);
            continue;
        }
        match b {
            b'(' | b'[' | b'{' => delta += 1,
            b')' | b']' | b'}' => delta -= 1,
            _ => {}
        }
        last_code_byte = Some(b);
        i += 1;
    }

    (delta, last_code_byte == Some(b':'))
}

/// HTML / XML tag-aware indentation formatter
pub fn format_markup_indentation(source: &str, indent_size: usize, is_html: bool) -> String {
    let mut out = String::with_capacity(source.len() + 64);
    let mut indent_level: i32 = 0;
    let mut consecutive_blank_lines = 0usize;

    for raw_line in source.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            consecutive_blank_lines += 1;
            if consecutive_blank_lines <= 2 {
                out.push('\n');
            }
            continue;
        }
        consecutive_blank_lines = 0;

        let starts_with_closing = trimmed.starts_with("</");
        let (opens, closes) = count_markup_tags(trimmed, is_html);

        let line_level = if starts_with_closing {
            (indent_level - 1).max(0)
        } else {
            indent_level.max(0)
        };

        out.push_str(&" ".repeat((line_level as usize) * indent_size));
        out.push_str(trimmed);
        out.push('\n');

        indent_level = (indent_level + opens - closes).max(0);
    }

    preserve_trailing_newline_preference(source, out)
}

fn count_markup_tags(line: &str, is_html: bool) -> (i32, i32) {
    const HTML_VOID_TAGS: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr",
    ];

    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut opens = 0i32;
    let mut closes = 0i32;

    while i < len {
        if bytes[i] == b'<' {
            // Comment <!-- ... --> or <!DOCTYPE ...> or <?xml ...?>
            if i + 1 < len && (bytes[i + 1] == b'!' || bytes[i + 1] == b'?') {
                if let Some(rel_end) = line[i..].find('>') {
                    i += rel_end + 1;
                    continue;
                } else {
                    break;
                }
            }

            if let Some(rel_end) = line[i..].find('>') {
                let tag_slice = &line[i + 1..i + rel_end];
                let tag_trimmed = tag_slice.trim();
                if tag_trimmed.starts_with('/') {
                    closes += 1;
                } else if !tag_trimmed.ends_with('/') {
                    let tag_name = tag_trimmed
                        .split(|c: char| c.is_ascii_whitespace() || c == '/')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if !(is_html && HTML_VOID_TAGS.contains(&tag_name.as_str()))
                        && !tag_name.is_empty()
                    {
                        opens += 1;
                    }
                }
                i += rel_end + 1;
                continue;
            }
        }
        i += 1;
    }

    (opens, closes)
}

/// Shell / Arch PKGBUILD smart indenter
pub fn format_shell_indentation(source: &str, indent_size: usize) -> String {
    let mut out = String::with_capacity(source.len() + 64);
    let mut indent_level: i32 = 0;
    let mut consecutive_blank_lines = 0usize;

    for raw_line in source.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            consecutive_blank_lines += 1;
            if consecutive_blank_lines <= 2 {
                out.push('\n');
            }
            continue;
        }
        consecutive_blank_lines = 0;

        let code_part = strip_hash_comment(trimmed);
        let dedent_now = code_part == "fi"
            || code_part.starts_with("fi;")
            || code_part.starts_with("fi ")
            || code_part == "done"
            || code_part.starts_with("done;")
            || code_part == "esac"
            || code_part == "else"
            || code_part.starts_with("elif ")
            || code_part.starts_with('}')
            || code_part.starts_with(')');

        let line_level = if dedent_now {
            (indent_level - 1).max(0)
        } else {
            indent_level.max(0)
        };

        out.push_str(&" ".repeat((line_level as usize) * indent_size));
        out.push_str(trimmed);
        out.push('\n');

        let mut delta = 0i32;
        if code_part == "fi"
            || code_part.starts_with("fi;")
            || code_part == "done"
            || code_part.starts_with("done;")
            || code_part == "esac"
        {
            delta -= 1;
        }
        if code_part.ends_with("then")
            || code_part.ends_with("do")
            || code_part == "else"
            || code_part.starts_with("case ") && code_part.ends_with(" in")
        {
            if !(code_part.starts_with("elif ") || code_part == "else") {
                delta += 1;
            }
        }
        if code_part.ends_with('{') || code_part.ends_with('(') {
            delta += 1;
        }
        if code_part.starts_with('}') || code_part.starts_with(')') {
            delta -= 1;
        }

        indent_level = (indent_level + delta).max(0);
    }

    preserve_trailing_newline_preference(source, out)
}

/// Lua smart indenter
pub fn format_lua_indentation(source: &str, indent_size: usize) -> String {
    let mut out = String::with_capacity(source.len() + 64);
    let mut indent_level: i32 = 0;
    let mut consecutive_blank_lines = 0usize;

    for raw_line in source.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            consecutive_blank_lines += 1;
            if consecutive_blank_lines <= 2 {
                out.push('\n');
            }
            continue;
        }
        consecutive_blank_lines = 0;

        let code = trimmed.split("--").next().unwrap_or("").trim();
        let dedent_now = code == "end"
            || code.starts_with("end)")
            || code.starts_with("end,")
            || code == "else"
            || code.starts_with("elseif ")
            || code.starts_with("until ")
            || code.starts_with('}');

        let line_level = if dedent_now {
            (indent_level - 1).max(0)
        } else {
            indent_level.max(0)
        };

        out.push_str(&" ".repeat((line_level as usize) * indent_size));
        out.push_str(trimmed);
        out.push('\n');

        let mut delta = 0i32;
        if code == "end"
            || code.starts_with("end)")
            || code.starts_with("end,")
            || code.starts_with("until ")
            || code.starts_with('}')
        {
            delta -= 1;
        }
        let opens_block = (code.starts_with("function ")
            || code.contains("= function(")
            || code.starts_with("local function ")
            || code.ends_with(" then")
            || code == "do"
            || code.ends_with(" do")
            || code == "repeat")
            && !code.ends_with(" end")
            && !code.starts_with("elseif ");
        if opens_block || code.ends_with('{') {
            delta += 1;
        }

        indent_level = (indent_level + delta).max(0);
    }

    preserve_trailing_newline_preference(source, out)
}

fn strip_hash_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if !in_single => {
                i += 2;
                continue;
            }
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'#' if !in_single && !in_double => {
                if i == 0 || bytes[i - 1].is_ascii_whitespace() {
                    return line[..i].trim();
                }
            }
            _ => {}
        }
        i += 1;
    }
    line.trim()
}

fn normalize_whitespace_and_tabs(source: &str, indent_size: usize) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.is_empty() {
            out.push('\n');
            continue;
        }
        let cols = visual_indent_columns(trimmed_end, indent_size);
        let content = trimmed_end.trim_start_matches([' ', '\t']);
        out.push_str(&" ".repeat(cols));
        out.push_str(content);
        out.push('\n');
    }
    preserve_trailing_newline_preference(source, out)
}

fn preserve_trailing_newline_preference(original: &str, mut formatted: String) -> String {
    if !original.ends_with('\n') && formatted.ends_with('\n') {
        formatted.pop();
    }
    formatted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_messy_c_code_ignoring_braces_in_strings_and_comments() {
        let messy = "\
int main() {
if (x > 0) {
// ignore { unmatched brace in comment
printf(\"hello { world }\\n\");
return 1;
}else{
return 0;
}
}";
        let expected = "\
int main() {
    if (x > 0) {
        // ignore { unmatched brace in comment
        printf(\"hello { world }\\n\");
        return 1;
    } else {
        return 0;
    }
}";
        assert_eq!(format_brace_indentation(messy, 4), expected);
    }

    #[test]
    fn formats_python_blocks_and_else_clauses() {
        let messy = "\
def greet(name):
if name:
\t  print('hi', name)
else:
  print('anon')
";
        let expected = "\
def greet(name):
    if name:
        print('hi', name)
    else:
        print('anon')
";
        assert_eq!(format_python_indentation(messy, 4), expected);
    }

    #[test]
    fn formats_html_nested_tags_and_void_elements() {
        let messy = "\
<div>
<img src=\"logo.png\">
<ul>
<li>Item 1</li>
</ul>
</div>";
        let expected = "\
<div>
  <img src=\"logo.png\">
  <ul>
    <li>Item 1</li>
  </ul>
</div>";
        assert_eq!(format_markup_indentation(messy, 2, true), expected);
    }

    #[test]
    fn formats_json_pretty() {
        let raw = r#"{"app":"litecode","features":["editor","auto-format"]}"#;
        let outcome = format_code(
            raw,
            Language::Json,
            None,
            FormatOptions {
                indent_size: 2,
                use_system_formatters: false,
            },
        );
        assert!(outcome.changed);
        assert!(outcome.formatted.contains("\n  \"app\": \"litecode\""));
    }

    #[test]
    fn formats_switch_case_and_block_comments() {
        let messy = "\
/*
* Header doc
*/
switch (mode) {
case 1:
do_one();
break;
default:
do_default();
}";
        let expected = "\
/*
 * Header doc
 */
switch (mode) {
    case 1:
        do_one();
        break;
    default:
        do_default();
}";
        assert_eq!(format_brace_indentation(messy, 4), expected);
    }

    #[test]
    fn formats_shell_and_lua_blocks() {
        let messy_sh = "\
build() {
if [ -f Cargo.toml ]; then
cargo build --release
else
echo missing
fi
}";
        let expected_sh = "\
build() {
    if [ -f Cargo.toml ]; then
        cargo build --release
    else
        echo missing
    fi
}";
        assert_eq!(format_shell_indentation(messy_sh, 4), expected_sh);

        let messy_lua = "\
local function test(x)
if x > 0 then
print(x)
else
print(0)
end
end";
        let expected_lua = "\
local function test(x)
    if x > 0 then
        print(x)
    else
        print(0)
    end
end";
        assert_eq!(format_lua_indentation(messy_lua, 4), expected_lua);
    }

    #[test]
    fn uses_system_rustfmt_when_enabled() {
        let messy_rs = "fn main(){let x=1;if x>0{println!(\"{}\",x);}}";
        let outcome = format_code(
            messy_rs,
            Language::Rust,
            None,
            FormatOptions {
                indent_size: 4,
                use_system_formatters: true,
            },
        );
        assert!(outcome.changed);
        assert!(outcome.engine_used.contains("rustfmt"));
        assert!(outcome.formatted.contains("    let x = 1;\n"));
    }
}
