use gtk4::prelude::*;
use gtk4::{TextBuffer, TextTag};

use crate::language::Language;

const MAX_HIGHLIGHT_CHARS: usize = 200_000;

pub fn install_syntax_tags(buffer: &TextBuffer) {
    let table = buffer.tag_table();
    let tags = [
        ("syn_comment", "#6a9955"),
        ("syn_string", "#ce9178"),
        ("syn_keyword", "#569cd6"),
        ("syn_control", "#c586c0"),
        ("syn_type", "#4ec9b0"),
        ("syn_func", "#dcdcaa"),
        ("syn_number", "#b5cea8"),
        ("syn_meta", "#9cdcfe"),
    ];

    for (name, color) in tags {
        if table.lookup(name).is_none() {
            let tag = TextTag::builder()
                .name(name)
                .foreground(color)
                .build();
            table.add(&tag);
        }
    }
}

pub fn highlight_buffer(buffer: &TextBuffer, lang: Language) {
    let start_iter = buffer.start_iter();
    let end_iter = buffer.end_iter();

    for tag_name in [
        "syn_comment",
        "syn_string",
        "syn_keyword",
        "syn_control",
        "syn_type",
        "syn_func",
        "syn_number",
        "syn_meta",
    ] {
        buffer.remove_tag_by_name(tag_name, &start_iter, &end_iter);
    }

    if lang == Language::PlainText {
        return;
    }

    let text = buffer.text(&start_iter, &end_iter, false);
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() || chars.len() > MAX_HIGHLIGHT_CHARS {
        return;
    }

    match lang {
        Language::Html | Language::Xml => highlight_markup(buffer, &chars),
        Language::Json => highlight_json(buffer, &chars),
        Language::Markdown => highlight_markdown(buffer, &chars),
        Language::Toml | Language::Yaml => highlight_config(buffer, &chars),
        _ => highlight_code(buffer, &chars, lang),
    }
}

#[inline]
fn apply_tag(buffer: &TextBuffer, name: &str, start: usize, end: usize) {
    if start < end {
        let s = buffer.iter_at_offset(start as i32);
        let e = buffer.iter_at_offset(end as i32);
        buffer.apply_tag_by_name(name, &s, &e);
    }
}

fn highlight_code(buffer: &TextBuffer, chars: &[char], lang: Language) {
    let n = chars.len();
    let mut i = 0usize;
    let mut at_line_start = true;

    let hash_comments = matches!(lang, Language::Python | Language::Shell);
    let dash_comments = lang == Language::Lua;
    let slash_comments = matches!(
        lang,
        Language::Rust
            | Language::C
            | Language::Cpp
            | Language::JavaScript
            | Language::TypeScript
            | Language::Go
            | Language::Java
            | Language::Css
    );

    while i < n {
        let ch = chars[i];

        if ch == '\n' {
            at_line_start = true;
            i += 1;
            continue;
        }
        if ch.is_whitespace() {
            i += 1;
            continue;
        }

        // Line comments: //
        if slash_comments && ch == '/' && i + 1 < n && chars[i + 1] == '/' {
            let start = i;
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            apply_tag(buffer, "syn_comment", start, i);
            continue;
        }

        // Block comments: /* ... */
        if slash_comments && ch == '/' && i + 1 < n && chars[i + 1] == '*' {
            let start = i;
            i += 2;
            while i < n {
                if chars[i] == '*' && i + 1 < n && chars[i + 1] == '/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            apply_tag(buffer, "syn_comment", start, i);
            at_line_start = false;
            continue;
        }

        // Lua comments: -- and --[[ ... ]]
        if dash_comments && ch == '-' && i + 1 < n && chars[i + 1] == '-' {
            let start = i;
            if i + 3 < n && chars[i + 2] == '[' && chars[i + 3] == '[' {
                i += 4;
                while i < n {
                    if chars[i] == ']' && i + 1 < n && chars[i + 1] == ']' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            } else {
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
            }
            apply_tag(buffer, "syn_comment", start, i);
            continue;
        }

        // Hash comments (#) vs C preprocessor / Rust #[...] attributes
        if ch == '#' {
            if hash_comments {
                let start = i;
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
                apply_tag(buffer, "syn_comment", start, i);
                continue;
            }
            if lang == Language::Rust && i + 1 < n && (chars[i + 1] == '[' || chars[i + 1] == '!') {
                let start = i;
                let mut depth = 0i32;
                while i < n {
                    if chars[i] == '[' {
                        depth += 1;
                    } else if chars[i] == ']' {
                        depth -= 1;
                        if depth <= 0 {
                            i += 1;
                            break;
                        }
                    }
                    i += 1;
                }
                apply_tag(buffer, "syn_meta", start, i);
                at_line_start = false;
                continue;
            }
            if matches!(lang, Language::C | Language::Cpp) && at_line_start {
                let start = i;
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
                apply_tag(buffer, "syn_meta", start, i);
                continue;
            }
        }

        // Decorators (@...) in Python / TypeScript / Java
        if ch == '@' && i + 1 < n && (chars[i + 1].is_alphabetic() || chars[i + 1] == '_') {
            let start = i;
            i += 1;
            while i < n && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            apply_tag(buffer, "syn_meta", start, i);
            at_line_start = false;
            continue;
        }

        // Strings & char literals (watch out for Rust lifetimes `'a`)
        if ch == '"' || ch == '\'' || ch == '`' {
            if lang == Language::Rust && ch == '\'' {
                // Check if this is a lifetime like 'static or 'a rather than 'x'
                let is_char_lit = (i + 2 < n && chars[i + 2] == '\'')
                    || (i + 3 < n && chars[i + 1] == '\\' && chars[i + 3] == '\'');
                if !is_char_lit {
                    let start = i;
                    i += 1;
                    while i < n && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    apply_tag(buffer, "syn_meta", start, i);
                    at_line_start = false;
                    continue;
                }
            }

            let start = i;
            let quote = ch;
            // Python triple-quoted strings
            if lang == Language::Python && i + 2 < n && chars[i + 1] == quote && chars[i + 2] == quote
            {
                i += 3;
                while i < n {
                    if chars[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if i + 2 < n && chars[i] == quote && chars[i + 1] == quote && chars[i + 2] == quote
                    {
                        i += 3;
                        break;
                    }
                    i += 1;
                }
            } else {
                i += 1;
                while i < n {
                    if chars[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if chars[i] == quote || (quote != '`' && chars[i] == '\n') {
                        if chars[i] == quote {
                            i += 1;
                        }
                        break;
                    }
                    i += 1;
                }
            }
            apply_tag(buffer, "syn_string", start, i);
            at_line_start = false;
            continue;
        }

        // Numbers
        if ch.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < n
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                i += 1;
            }
            apply_tag(buffer, "syn_number", start, i);
            at_line_start = false;
            continue;
        }

        // Identifiers & Keywords
        if ch.is_alphabetic() || ch == '_' || (lang == Language::Css && ch == '-') {
            let start = i;
            i += 1;
            while i < n
                && (chars[i].is_alphanumeric()
                    || chars[i] == '_'
                    || (lang == Language::Css && chars[i] == '-'))
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();

            // Rust macro like println!(...) or vec![...]
            if lang == Language::Rust && i < n && chars[i] == '!' {
                apply_tag(buffer, "syn_func", start, i + 1);
                i += 1;
                at_line_start = false;
                continue;
            }

            if is_control_keyword(&word, lang) {
                apply_tag(buffer, "syn_control", start, i);
            } else if is_language_keyword(&word, lang) {
                apply_tag(buffer, "syn_keyword", start, i);
            } else if is_builtin_or_pascal_type(&word) {
                apply_tag(buffer, "syn_type", start, i);
            } else {
                // Function call if followed by '('
                let mut peek = i;
                while peek < n && (chars[peek] == ' ' || chars[peek] == '\t') {
                    peek += 1;
                }
                if peek < n && chars[peek] == '(' {
                    apply_tag(buffer, "syn_func", start, i);
                }
            }

            at_line_start = false;
            continue;
        }

        at_line_start = false;
        i += 1;
    }
}

fn highlight_json(buffer: &TextBuffer, chars: &[char]) {
    let n = chars.len();
    let mut i = 0usize;
    while i < n {
        let ch = chars[i];
        if ch == '/' && i + 1 < n && chars[i + 1] == '/' {
            let start = i;
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            apply_tag(buffer, "syn_comment", start, i);
            continue;
        }
        if ch == '"' {
            let start = i;
            i += 1;
            while i < n {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            let mut peek = i;
            while peek < n && chars[peek].is_whitespace() {
                peek += 1;
            }
            if peek < n && chars[peek] == ':' {
                apply_tag(buffer, "syn_meta", start, i);
            } else {
                apply_tag(buffer, "syn_string", start, i);
            }
            continue;
        }
        if ch.is_ascii_digit() || (ch == '-' && i + 1 < n && chars[i + 1].is_ascii_digit()) {
            let start = i;
            i += 1;
            while i < n
                && (chars[i].is_ascii_digit()
                    || chars[i] == '.'
                    || chars[i] == 'e'
                    || chars[i] == 'E'
                    || chars[i] == '+'
                    || chars[i] == '-')
            {
                i += 1;
            }
            apply_tag(buffer, "syn_number", start, i);
            continue;
        }
        if ch.is_ascii_alphabetic() {
            let start = i;
            while i < n && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if matches!(word.as_str(), "true" | "false" | "null") {
                apply_tag(buffer, "syn_keyword", start, i);
            }
            continue;
        }
        i += 1;
    }
}

fn highlight_markup(buffer: &TextBuffer, chars: &[char]) {
    let n = chars.len();
    let mut i = 0usize;
    while i < n {
        if chars[i] == '<' {
            // Comment <!-- ... -->
            if i + 3 < n && chars[i + 1] == '!' && chars[i + 2] == '-' && chars[i + 3] == '-' {
                let start = i;
                i += 4;
                while i < n {
                    if i + 2 < n && chars[i] == '-' && chars[i + 1] == '-' && chars[i + 2] == '>' {
                        i += 3;
                        break;
                    }
                    i += 1;
                }
                apply_tag(buffer, "syn_comment", start, i);
                continue;
            }

            // Tag <tag attr="val">
            i += 1;
            if i < n && (chars[i] == '/' || chars[i] == '!' || chars[i] == '?') {
                i += 1;
            }
            let tag_start = i;
            while i < n
                && (chars[i].is_alphanumeric()
                    || chars[i] == '-'
                    || chars[i] == '_'
                    || chars[i] == ':')
            {
                i += 1;
            }
            apply_tag(buffer, "syn_keyword", tag_start, i);

            while i < n && chars[i] != '>' {
                if chars[i] == '"' || chars[i] == '\'' {
                    let q = chars[i];
                    let s_start = i;
                    i += 1;
                    while i < n && chars[i] != q {
                        i += 1;
                    }
                    if i < n {
                        i += 1;
                    }
                    apply_tag(buffer, "syn_string", s_start, i);
                    continue;
                }
                if chars[i].is_alphabetic() || chars[i] == '-' {
                    let a_start = i;
                    while i < n
                        && (chars[i].is_alphanumeric()
                            || chars[i] == '-'
                            || chars[i] == '_'
                            || chars[i] == ':')
                    {
                        i += 1;
                    }
                    apply_tag(buffer, "syn_meta", a_start, i);
                    continue;
                }
                i += 1;
            }
        }
        i += 1;
    }
}

fn highlight_config(buffer: &TextBuffer, chars: &[char]) {
    let n = chars.len();
    let mut i = 0usize;
    let mut line_start = true;

    while i < n {
        let ch = chars[i];
        if ch == '\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if ch == '#' {
            let start = i;
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            apply_tag(buffer, "syn_comment", start, i);
            continue;
        }
        if line_start && ch == '[' {
            let start = i;
            while i < n && chars[i] != '\n' && chars[i] != '#' {
                i += 1;
            }
            apply_tag(buffer, "syn_type", start, i);
            line_start = false;
            continue;
        }
        if ch == '"' || ch == '\'' {
            let q = ch;
            let start = i;
            i += 1;
            while i < n && chars[i] != q && chars[i] != '\n' {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            if i < n && chars[i] == q {
                i += 1;
            }
            apply_tag(buffer, "syn_string", start, i);
            line_start = false;
            continue;
        }
        if line_start && (ch.is_alphanumeric() || ch == '_' || ch == '-') {
            let start = i;
            while i < n && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '-' || chars[i] == '.') {
                i += 1;
            }
            apply_tag(buffer, "syn_meta", start, i);
            line_start = false;
            continue;
        }
        if ch.is_ascii_digit() {
            let start = i;
            while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '-' || chars[i] == '_') {
                i += 1;
            }
            apply_tag(buffer, "syn_number", start, i);
            line_start = false;
            continue;
        }
        if !ch.is_whitespace() {
            line_start = false;
        }
        i += 1;
    }
}

fn highlight_markdown(buffer: &TextBuffer, chars: &[char]) {
    let n = chars.len();
    let mut i = 0usize;
    let mut line_start = true;

    while i < n {
        let ch = chars[i];
        if ch == '\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if line_start && ch == '#' {
            let start = i;
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            apply_tag(buffer, "syn_keyword", start, i);
            continue;
        }
        if ch == '`' {
            let start = i;
            if i + 2 < n && chars[i + 1] == '`' && chars[i + 2] == '`' {
                i += 3;
                while i < n {
                    if i + 2 < n && chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
                        i += 3;
                        break;
                    }
                    i += 1;
                }
            } else {
                i += 1;
                while i < n && chars[i] != '`' && chars[i] != '\n' {
                    i += 1;
                }
                if i < n && chars[i] == '`' {
                    i += 1;
                }
            }
            apply_tag(buffer, "syn_string", start, i);
            line_start = false;
            continue;
        }
        if !ch.is_whitespace() {
            line_start = false;
        }
        i += 1;
    }
}

fn is_control_keyword(word: &str, _lang: Language) -> bool {
    matches!(
        word,
        "if" | "else"
            | "elif"
            | "elseif"
            | "then"
            | "fi"
            | "match"
            | "switch"
            | "case"
            | "default"
            | "esac"
            | "for"
            | "while"
            | "loop"
            | "do"
            | "done"
            | "until"
            | "repeat"
            | "in"
            | "return"
            | "break"
            | "continue"
            | "try"
            | "catch"
            | "except"
            | "finally"
            | "throw"
            | "raise"
            | "yield"
            | "await"
            | "import"
            | "from"
            | "export"
            | "use"
            | "where"
            | "with"
    )
}

fn is_language_keyword(word: &str, _lang: Language) -> bool {
    matches!(
        word,
        "fn" | "pub"
            | "let"
            | "mut"
            | "const"
            | "static"
            | "struct"
            | "enum"
            | "impl"
            | "trait"
            | "type"
            | "mod"
            | "crate"
            | "super"
            | "self"
            | "async"
            | "move"
            | "ref"
            | "unsafe"
            | "extern"
            | "dyn"
            | "def"
            | "class"
            | "lambda"
            | "pass"
            | "global"
            | "nonlocal"
            | "and"
            | "or"
            | "not"
            | "is"
            | "None"
            | "True"
            | "False"
            | "function"
            | "var"
            | "new"
            | "delete"
            | "this"
            | "typeof"
            | "instanceof"
            | "interface"
            | "implements"
            | "extends"
            | "public"
            | "private"
            | "protected"
            | "virtual"
            | "override"
            | "namespace"
            | "using"
            | "template"
            | "typename"
            | "sizeof"
            | "typedef"
            | "package"
            | "func"
            | "defer"
            | "go"
            | "chan"
            | "select"
            | "local"
            | "end"
            | "nil"
            | "true"
            | "false"
            | "null"
            | "undefined"
    )
}

fn is_builtin_or_pascal_type(word: &str) -> bool {
    if matches!(
        word,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
            | "int"
            | "long"
            | "short"
            | "float"
            | "double"
            | "void"
            | "signed"
            | "unsigned"
            | "size_t"
            | "ssize_t"
            | "uint8_t"
            | "uint16_t"
            | "uint32_t"
            | "uint64_t"
            | "int32_t"
            | "int64_t"
            | "string"
            | "number"
            | "boolean"
            | "any"
            | "never"
            | "Self"
    ) {
        return true;
    }

    let mut chars = word.chars();
    if let Some(first) = chars.next() {
        if first.is_ascii_uppercase() && word.chars().any(|c| c.is_ascii_lowercase()) {
            return true;
        }
    }
    false
}
