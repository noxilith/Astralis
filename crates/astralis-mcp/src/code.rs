//! Cheap code navigation for AI agents: `outline` (the definitions of one file) and `find`
//! (where an identifier is defined across the project). Both return tiny plain-text answers so
//! an agent doesn't have to read whole files just to learn their shape or locate a symbol.
//!
//! Pure std, line-based recognition (no parser, no regex): good enough to be useful, never
//! perfect. Comment lines and block comments are skipped; everything else is best effort.

use std::fs;
use std::path::{Component, Path, PathBuf};

/// Largest file `outline` will read.
const MAX_OUTLINE_BYTES: u64 = 2 * 1024 * 1024;
/// Largest file `find` will scan (bigger ones are generated/vendored more often than not).
const MAX_FIND_BYTES: u64 = 512 * 1024;
/// Most source files `find` reads before giving up.
const MAX_SCANNED_FILES: usize = 4000;
/// Signatures are cut to this many characters.
const SIG_MAX: usize = 140;
/// Directories `find` never enters (every dot-directory is skipped as well).
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", "out", "vendor", "coverage", "__pycache__", "venv",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Rust,
    Script, // TypeScript / JavaScript
    Python,
    Go,
    Jvm, // Java, Kotlin, C#, Swift, PHP
    C,   // C, C++ and headers
    Ruby,
    Shell,
    Markdown,
    Toml,
    Yaml,
    Json,
    Sql,
}

/// The language of a file, by extension (None = not a source file we understand).
fn lang_of(path: &Path) -> Option<Lang> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => Lang::Rust,
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" => Lang::Script,
        "py" => Lang::Python,
        "go" => Lang::Go,
        "java" | "kt" | "kts" | "cs" | "swift" | "php" => Lang::Jvm,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Lang::C,
        "rb" => Lang::Ruby,
        "sh" | "bash" | "zsh" => Lang::Shell,
        "md" | "markdown" => Lang::Markdown,
        "toml" => Lang::Toml,
        "yaml" | "yml" => Lang::Yaml,
        "json" => Lang::Json,
        "sql" => Lang::Sql,
        _ => return None,
    })
}

/// A friendly language name for the header line.
fn label(path: &Path, lang: Lang) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match (lang, ext.as_str()) {
        (Lang::Rust, _) => "Rust",
        (Lang::Script, "ts" | "tsx" | "mts" | "cts") => "TypeScript",
        (Lang::Script, _) => "JavaScript",
        (Lang::Python, _) => "Python",
        (Lang::Go, _) => "Go",
        (Lang::Jvm, "java") => "Java",
        (Lang::Jvm, "kt" | "kts") => "Kotlin",
        (Lang::Jvm, "cs") => "C#",
        (Lang::Jvm, "swift") => "Swift",
        (Lang::Jvm, _) => "PHP",
        (Lang::C, "c" | "h") => "C",
        (Lang::C, _) => "C++",
        (Lang::Ruby, _) => "Ruby",
        (Lang::Shell, _) => "Shell",
        (Lang::Markdown, _) => "Markdown",
        (Lang::Toml, _) => "TOML",
        (Lang::Yaml, _) => "YAML",
        (Lang::Json, _) => "JSON",
        (Lang::Sql, _) => "SQL",
    }
}

// ---------------------------------------------------------------------------------------------
// Small string helpers
// ---------------------------------------------------------------------------------------------

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The identifier at the start of `s` (possibly empty).
fn ident(s: &str) -> &str {
    let end = s.char_indices().find(|&(_, c)| !is_ident_char(c)).map(|(i, _)| i).unwrap_or(s.len());
    &s[..end]
}

/// Like `ident` but also keeps `::` path separators (`a::Foo`).
fn path_ident(s: &str) -> &str {
    let end = s
        .char_indices()
        .find(|&(_, c)| !(is_ident_char(c) || c == ':'))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    &s[..end]
}

/// `s` without the leading word `w` (whole word only), with following whitespace trimmed.
fn strip_word<'a>(s: &'a str, w: &str) -> Option<&'a str> {
    let r = s.strip_prefix(w)?;
    match r.chars().next() {
        None => Some(r),
        Some(c) if is_ident_char(c) => None,
        Some(_) => Some(r.trim_start()),
    }
}

/// Skip a leading `<...>` generics list (balanced), returning what follows.
fn skip_generics(s: &str) -> &str {
    if !s.starts_with('<') {
        return s;
    }
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return s[i + 1..].trim_start();
                }
            }
            _ => {}
        }
    }
    s
}

fn nz(s: &str) -> Option<String> {
    if s.is_empty() { None } else { Some(s.to_string()) }
}

/// Truncate to at most `max` characters (on a char boundary), marking the cut with `…`.
fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// The tidy one-line signature of a definition line: whitespace collapsed, the body (the first
/// `{` outside parentheses/brackets) cut off, length capped.
fn clean_sig(line: &str) -> String {
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut depth = 0i32;
    let mut end = collapsed.len();
    for (i, c) in collapsed.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '{' if depth <= 0 => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    cut(collapsed[..end].trim_end(), SIG_MAX)
}

// ---------------------------------------------------------------------------------------------
// Per-language definition recognisers: each takes one trimmed, non-comment line and returns the
// declared name if the line is a definition.
// ---------------------------------------------------------------------------------------------

fn rust_def(t: &str) -> Option<String> {
    let mut s = t;
    if let Some(r) = s.strip_prefix("pub") {
        if r.starts_with('(') {
            let i = r.find(')')?;
            s = r[i + 1..].trim_start();
        } else if r.starts_with(char::is_whitespace) {
            s = r.trim_start();
        }
    }
    // Modifiers: async / unsafe / default / const (only before fn-ish words) / extern "C".
    loop {
        let before = s.len();
        for w in ["async", "unsafe", "default"] {
            if let Some(r) = strip_word(s, w) {
                s = r;
            }
        }
        if let Some(r) = strip_word(s, "const") {
            if ["fn", "async", "unsafe", "extern"].iter().any(|w| strip_word(r, w).is_some()) {
                s = r;
            }
        }
        if let Some(r) = strip_word(s, "extern") {
            s = r;
            if s.starts_with('"') {
                if let Some(i) = s[1..].find('"') {
                    s = s[i + 2..].trim_start();
                }
            }
        }
        if s.len() == before {
            break;
        }
    }
    if let Some(r) = s.strip_prefix("macro_rules!") {
        return nz(ident(r.trim_start()));
    }
    if let Some(r) = strip_word(s, "impl") {
        let mut h = skip_generics(r);
        if let Some(i) = h.find(" for ") {
            h = h[i + 5..].trim_start();
        }
        let h = h.trim_start_matches(['&', '!']).trim_start();
        let h = strip_word(h, "dyn").unwrap_or(h);
        return nz(path_ident(h).rsplit("::").next().unwrap_or(""));
    }
    for kw in ["fn", "struct", "enum", "trait", "union", "mod", "type", "static", "const"] {
        if let Some(r) = strip_word(s, kw) {
            let r = if kw == "static" { strip_word(r, "mut").unwrap_or(r) } else { r };
            return nz(ident(r));
        }
    }
    None
}

/// TypeScript/JavaScript. Returns (name, is_class).
fn script_def(t: &str, indent: usize, in_class: bool) -> Option<(String, bool)> {
    let mut s = t;
    let mut exported = false;
    loop {
        let before = s.len();
        if let Some(r) = strip_word(s, "export") {
            s = r;
            exported = true;
        }
        for w in ["default", "declare", "abstract"] {
            if let Some(r) = strip_word(s, w) {
                s = r;
            }
        }
        if s.len() == before {
            break;
        }
    }
    let a = strip_word(s, "async").unwrap_or(s);
    if let Some(r) = strip_word(a, "function") {
        let r = r.trim_start_matches('*').trim_start();
        return nz(ident(r)).map(|n| (n, false));
    }
    if let Some(r) = strip_word(s, "class") {
        return nz(ident(r)).map(|n| (n, true));
    }
    for kw in ["interface", "namespace"] {
        if let Some(r) = strip_word(s, kw) {
            return nz(ident(r)).map(|n| (n, false));
        }
    }
    let en = strip_word(s, "const").and_then(|r| strip_word(r, "enum")).or_else(|| strip_word(s, "enum"));
    if let Some(r) = en {
        return nz(ident(r)).map(|n| (n, false));
    }
    if let Some(r) = strip_word(s, "type") {
        let n = ident(r);
        let after = r[n.len()..].trim_start();
        if !n.is_empty() && (after.starts_with('=') || after.starts_with('<')) {
            return Some((n.to_string(), false));
        }
        return None;
    }
    for kw in ["const", "let", "var"] {
        if let Some(r) = strip_word(s, kw) {
            let n = ident(r);
            if n.is_empty() {
                return None;
            }
            let after = r[n.len()..].trim_start();
            let after = if after.starts_with(':') { &after[after.find('=')?..] } else { after };
            let rhs = after.strip_prefix('=')?.trim_start();
            let arrow = rhs.contains("=>");
            let upper = n.starts_with(char::is_uppercase);
            let fnlike = rhs.starts_with("function")
                || (rhs.starts_with("async")
                    && (arrow || rhs[5..].trim_start().starts_with("function")))
                || ((rhs.starts_with('(') || rhs.starts_with('<')) && arrow)
                || (arrow && {
                    let i = ident(rhs);
                    !i.is_empty() && rhs[i.len()..].trim_start().starts_with("=>")
                })
                || (upper
                    && ["memo(", "forwardRef(", "React.", "styled"].iter().any(|p| rhs.starts_with(p)));
            if fnlike && (exported || indent == 0 || upper) {
                return Some((n.to_string(), false));
            }
            return None;
        }
    }
    // Class members: `[modifiers] name(args) {` or `name = (…) => …`.
    if in_class && indent > 0 {
        let mut m = t;
        loop {
            let before = m.len();
            for w in ["public", "private", "protected", "static", "readonly", "async", "get", "set", "override", "abstract", "declare"] {
                if let Some(r) = strip_word(m, w) {
                    m = r;
                }
            }
            m = m.trim_start_matches('*').trim_start();
            if m.len() == before {
                break;
            }
        }
        let n = ident(m);
        const KW: &[&str] = &[
            "if", "for", "while", "switch", "catch", "function", "return", "else", "do", "try", "with",
            "typeof", "await", "new", "delete", "throw", "yield", "super",
        ];
        if n.is_empty() || KW.contains(&n) {
            return None;
        }
        let after = m[n.len()..].trim_start();
        if after.starts_with('(') || after.starts_with('<') {
            let opens = t.matches('(').count();
            if opens == t.matches(')').count() && t.ends_with('{') {
                return Some((n.to_string(), false));
            }
        } else if let Some(rhs) = after.strip_prefix('=') {
            if rhs.trim_start().starts_with(['(', 'a']) && rhs.contains("=>") {
                return Some((n.to_string(), false));
            }
        }
    }
    None
}

fn python_def(t: &str) -> Option<String> {
    let s = strip_word(t, "async").unwrap_or(t);
    for kw in ["def", "class"] {
        if let Some(r) = strip_word(s, kw) {
            return nz(ident(r));
        }
    }
    None
}

fn go_def(t: &str) -> Option<String> {
    if let Some(r) = strip_word(t, "func") {
        let r = if r.starts_with('(') { r[r.find(')')? + 1..].trim_start() } else { r };
        return nz(ident(r));
    }
    if let Some(r) = strip_word(t, "type") {
        let n = ident(r);
        if !n.is_empty() && !r[n.len()..].trim().is_empty() {
            return Some(n.to_string());
        }
    }
    None
}

fn ruby_def(t: &str) -> Option<String> {
    if let Some(r) = strip_word(t, "def") {
        let end = r.find(|c: char| c.is_whitespace() || c == '(').unwrap_or(r.len());
        return nz(&r[..end]);
    }
    for kw in ["class", "module"] {
        if let Some(r) = strip_word(t, kw) {
            let n = path_ident(r);
            if n.starts_with(char::is_alphabetic) {
                return Some(n.to_string());
            }
        }
    }
    None
}

fn shell_def(t: &str) -> Option<String> {
    let name_chars = |c: char| is_ident_char(c) || c == '-' || c == '.' || c == ':';
    if let Some(r) = strip_word(t, "function") {
        let end = r.find(|c: char| !name_chars(c)).unwrap_or(r.len());
        return nz(&r[..end]);
    }
    let end = t.find(|c: char| !name_chars(c)).unwrap_or(t.len());
    if end > 0 && t[end..].trim_start().starts_with("()") {
        return Some(t[..end].to_string());
    }
    None
}

const JVM_MODS: &[&str] = &[
    "public", "private", "protected", "internal", "static", "final", "abstract", "sealed", "open",
    "data", "partial", "override", "async", "virtual", "suspend", "inline", "readonly", "unsafe",
    "fileprivate", "synchronized", "native", "default", "extern", "lateinit", "operator", "infix",
    "tailrec", "external", "required", "convenience", "mutating", "nonisolated", "indirect",
    "annotation", "value", "inner", "const",
];
const STMT_WORDS: &[&str] = &[
    "return", "if", "else", "for", "while", "switch", "new", "throw", "catch", "try", "do", "case",
    "break", "continue", "yield", "await", "using", "lock", "foreach", "typeof", "delete", "goto",
    "assert", "sizeof",
];

/// Java / Kotlin / C# / Swift / PHP: type declarations, `fun|func|function`, and method-looking
/// lines (`[modifiers] Type name(…) {`).
fn jvm_def(t: &str) -> Option<String> {
    let mut s = t;
    while s.starts_with('@') {
        let end = s.find(char::is_whitespace).unwrap_or(s.len());
        s = s[end..].trim_start();
        if s.is_empty() {
            return None;
        }
    }
    let mut nmods = 0;
    loop {
        match JVM_MODS.iter().find_map(|m| strip_word(s, m)) {
            Some(r) => {
                s = r;
                nmods += 1;
            }
            None => break,
        }
    }
    if let Some(r) = strip_word(s, "fun") {
        if let Some(r2) = strip_word(r, "interface") {
            return nz(ident(r2));
        }
    }
    for kw in [
        "class", "interface", "enum", "struct", "record", "object", "trait", "protocol", "extension",
        "namespace",
    ] {
        if let Some(r) = strip_word(s, kw) {
            let r = strip_word(r, "class").or_else(|| strip_word(r, "interface")).unwrap_or(r);
            let end = r.find(|c: char| !(is_ident_char(c) || c == '.')).unwrap_or(r.len());
            let n = &r[..end];
            return if n.starts_with(char::is_alphabetic) { Some(n.to_string()) } else { None };
        }
    }
    for kw in ["fun", "func", "function"] {
        if let Some(r) = strip_word(s, kw) {
            let r = skip_generics(r.trim_start_matches('&').trim_start());
            let head = &r[..r.find('(')?];
            let n = head.trim().rsplit('.').next().unwrap_or("");
            return if !n.is_empty() && n.chars().all(is_ident_char) { Some(n.to_string()) } else { None };
        }
    }
    // Method-looking line.
    if !(t.ends_with('{') || t.ends_with(')') || t.contains(") throws ")) {
        return None;
    }
    let p = s.find('(')?;
    let prefix = &s[..p];
    if prefix.contains(['=', ';', '"', '.']) {
        return None;
    }
    let tokens: Vec<&str> = prefix.split_whitespace().collect();
    if tokens.iter().any(|w| STMT_WORDS.contains(w)) {
        return None;
    }
    let name = tokens.last()?.split('<').next()?.trim_start_matches(['*', '&']);
    if name.is_empty() || !name.chars().all(is_ident_char) || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    if tokens.len() >= 2 || nmods >= 1 { Some(name.to_string()) } else { None }
}

/// C / C++ / headers: only column-0 definitions (so calls inside bodies don't match).
fn c_def(t: &str, indent: usize) -> Option<String> {
    if indent != 0 || t.starts_with('}') || t.starts_with('{') {
        return None;
    }
    let mut s = t;
    loop {
        let before = s.len();
        for w in ["typedef", "static", "inline", "extern", "constexpr", "virtual", "friend", "explicit"] {
            if let Some(r) = strip_word(s, w) {
                s = r;
            }
        }
        if let Some(r) = strip_word(s, "template") {
            s = skip_generics(r);
        }
        if s.len() == before {
            break;
        }
    }
    if !t.contains('(') {
        for kw in ["struct", "class", "enum", "union", "namespace"] {
            if let Some(r) = strip_word(s, kw) {
                let r = strip_word(r, "class").or_else(|| strip_word(r, "struct")).unwrap_or(r);
                if t.ends_with(';') && !t.contains('{') {
                    return None; // forward declaration
                }
                return nz(ident(r));
            }
        }
        return None;
    }
    let p = s.find('(')?;
    let prefix = &s[..p];
    if prefix.contains(['=', ';', '"']) {
        return None;
    }
    let tokens: Vec<&str> = prefix.split_whitespace().collect();
    if tokens.iter().any(|w| STMT_WORDS.contains(w)) {
        return None;
    }
    let name = tokens.last()?.trim_start_matches(['*', '&']);
    let valid = !name.is_empty() && name.chars().all(|c| is_ident_char(c) || c == ':' || c == '~');
    let ends = t.ends_with('{') || t.ends_with(')') || t.ends_with(',') || t.ends_with('(');
    if valid && ends && (tokens.len() >= 2 || name.contains("::")) { Some(name.to_string()) } else { None }
}

fn sql_def(t: &str) -> Option<String> {
    let tokens: Vec<&str> = t.split_whitespace().collect();
    if !tokens.first()?.eq_ignore_ascii_case("create") {
        return None;
    }
    const SKIP: &[&str] = &[
        "or", "replace", "unique", "temp", "temporary", "materialized", "global", "local", "clustered",
        "nonclustered", "virtual",
    ];
    let mut i = 1;
    while i < tokens.len() && SKIP.iter().any(|w| tokens[i].eq_ignore_ascii_case(w)) {
        i += 1;
    }
    let kind = tokens.get(i)?.to_ascii_lowercase();
    if !["table", "view", "index", "function", "procedure", "trigger", "type", "sequence", "schema"].contains(&kind.as_str()) {
        return None;
    }
    i += 1;
    if tokens.get(i).is_some_and(|w| w.eq_ignore_ascii_case("if")) {
        i += 3; // if not exists
    }
    let raw = tokens.get(i)?;
    let name: String = raw
        .split(['(', ';'])
        .next()
        .unwrap_or("")
        .trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')
        .to_string();
    if name.is_empty() { None } else { Some(name) }
}

// ---------------------------------------------------------------------------------------------
// Parsing a whole file into definitions
// ---------------------------------------------------------------------------------------------

struct Def {
    line: usize,
    level: usize,
    name: String,
    sig: String,
}

fn leading_cols(l: &str) -> usize {
    let mut n = 0;
    for c in l.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += 4,
            _ => break,
        }
    }
    n
}

/// Markdown headings (outside fenced code), indented by heading level.
fn parse_markdown(text: &str) -> Vec<Def> {
    let mut out = Vec::new();
    let mut fenced = false;
    for (i, l) in text.lines().enumerate() {
        let t = l.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            let title = t[hashes..].trim().trim_end_matches('#').trim();
            if !title.is_empty() {
                out.push(Def { line: i + 1, level: hashes - 1, name: title.to_string(), sig: clean_sig(t) });
            }
        }
    }
    out
}

/// TOML / YAML / JSON: just the top-level sections or keys.
fn parse_data(lang: Lang, text: &str) -> Vec<Def> {
    let mut out = Vec::new();
    let mut in_section = false; // TOML: keys only count before the first [section]
    let json_indent = if lang == Lang::Json {
        text.lines().find(|l| l.trim_start().starts_with('"')).map(leading_cols)
    } else {
        None
    };
    for (i, l) in text.lines().enumerate() {
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let key = match lang {
            Lang::Toml => {
                if leading_cols(l) != 0 {
                    None
                } else if t.starts_with('[') {
                    in_section = true;
                    Some((t.trim_matches(|c| c == '[' || c == ']').trim().to_string(), clean_sig(t)))
                } else if !in_section {
                    t.split_once('=').map(|(k, _)| (k.trim().to_string(), k.trim().to_string()))
                } else {
                    None
                }
            }
            Lang::Yaml => {
                if leading_cols(l) != 0 || t.starts_with('-') || t.starts_with('%') {
                    None
                } else {
                    t.split_once(':').filter(|(k, _)| !k.is_empty() && !k.contains(' ')).map(|(k, _)| (k.to_string(), format!("{k}:")))
                }
            }
            _ => {
                if Some(leading_cols(l)) == json_indent && t.starts_with('"') {
                    let rest = &t[1..];
                    rest.find("\":").map(|e| (rest[..e].to_string(), format!("\"{}\"", &rest[..e])))
                } else {
                    None
                }
            }
        };
        if let Some((name, sig)) = key {
            if !name.is_empty() {
                out.push(Def { line: i + 1, level: 0, name, sig });
            }
        }
    }
    out
}

/// All definitions in `text`, in line order.
fn parse(lang: Lang, text: &str) -> Vec<Def> {
    match lang {
        Lang::Markdown => return parse_markdown(text),
        Lang::Toml | Lang::Yaml | Lang::Json => return parse_data(lang, text),
        _ => {}
    }
    let c_like = matches!(lang, Lang::Rust | Lang::Script | Lang::Jvm | Lang::C | Lang::Go | Lang::Sql);
    let mut in_block = false; // inside /* ... */
    let mut in_doc = false; // Python triple-quoted string
    let mut in_ruby = false; // =begin ... =end
    let mut class_indent: Option<usize> = None; // Script: indent of the enclosing class
    // (line, indent columns, name, signature)
    let mut raw: Vec<(usize, usize, String, String)> = Vec::new();

    for (i, l) in text.lines().enumerate() {
        let t = l.trim();
        if t.is_empty() {
            continue;
        }
        if c_like {
            if in_block {
                if t.contains("*/") {
                    in_block = false;
                }
                continue;
            }
            if let Some(rest) = t.strip_prefix("/*") {
                if !rest.contains("*/") {
                    in_block = true;
                }
                continue;
            }
        }
        if lang == Lang::Ruby {
            if l.starts_with("=begin") {
                in_ruby = true;
                continue;
            }
            if in_ruby {
                if l.starts_with("=end") {
                    in_ruby = false;
                }
                continue;
            }
        }
        let quotes = if lang == Lang::Python { t.matches("\"\"\"").count() + t.matches("'''").count() } else { 0 };
        if in_doc {
            if quotes % 2 == 1 {
                in_doc = false;
            }
            continue;
        }
        if t.starts_with("//") || t.starts_with('#') || t.starts_with('*') || t.starts_with("--") {
            continue;
        }
        let indent = leading_cols(l);
        if lang == Lang::Script {
            if let Some(ci) = class_indent {
                if indent <= ci {
                    class_indent = None;
                }
            }
        }
        let name = match lang {
            Lang::Rust => rust_def(t),
            Lang::Script => script_def(t, indent, class_indent.is_some()).map(|(n, is_class)| {
                if is_class {
                    class_indent = Some(indent);
                }
                n
            }),
            Lang::Python => python_def(t),
            Lang::Go => go_def(t),
            Lang::Jvm => jvm_def(t),
            Lang::C => c_def(t, indent),
            Lang::Ruby => ruby_def(t),
            Lang::Shell => shell_def(t),
            Lang::Sql => sql_def(t),
            _ => None,
        };
        if lang == Lang::Python && quotes % 2 == 1 {
            in_doc = true;
        }
        if let Some(name) = name {
            let sig = if lang == Lang::Sql {
                let c = t.split_whitespace().collect::<Vec<_>>().join(" ");
                cut(c.split(" (").next().unwrap_or(&c).trim_end_matches(['(', ';']), SIG_MAX)
            } else {
                clean_sig(t)
            };
            raw.push((i + 1, indent, name, sig));
        }
    }

    // Nesting: one level per indent unit, the unit being the smallest indent seen (2..=4).
    let unit = raw.iter().map(|r| r.1).filter(|&n| n > 0).min().unwrap_or(4).clamp(2, 4);
    raw.into_iter()
        .map(|(line, indent, name, sig)| Def { line, level: indent / unit, name, sig })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// File access
// ---------------------------------------------------------------------------------------------

/// True when a purely lexical walk of `p` climbs above its starting point.
fn lexically_escapes(p: &Path) -> bool {
    let mut depth = 0i32;
    for c in p.components() {
        match c {
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            _ => {}
        }
        if depth < 0 {
            return true;
        }
    }
    false
}

/// Resolve `path` (relative to `root`, or absolute) to a canonical file inside the project.
fn resolve(root: &Path, path: &str) -> Result<(PathBuf, PathBuf), String> {
    let root_c = root.canonicalize().map_err(|_| "cannot resolve the project root".to_string())?;
    let p = Path::new(path);
    let full = if p.is_absolute() { p.to_path_buf() } else { root_c.join(p) };
    match full.canonicalize() {
        Ok(c) => {
            if !c.starts_with(&root_c) {
                return Err("path is outside the project".into());
            }
            Ok((root_c, c))
        }
        Err(_) => {
            let outside = if p.is_absolute() { !full.starts_with(&root_c) } else { lexically_escapes(p) };
            Err(if outside { "path is outside the project".into() } else { format!("file not found: {path}") })
        }
    }
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|&b| b == 0)
}

fn rel_display(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

// ---------------------------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------------------------

/// The definitions of one file as a compact outline: a header line, then `L<line> <signature>`
/// per definition, nested ones indented two spaces per level, capped at `limit` lines.
pub fn outline(root: &Path, path: &str, limit: usize) -> Result<String, String> {
    let (root_c, file) = resolve(root, path)?;
    let meta = fs::metadata(&file).map_err(|e| format!("cannot read {path}: {e}"))?;
    if meta.is_dir() {
        return Err(format!("{path} is a directory, not a file"));
    }
    if meta.len() > MAX_OUTLINE_BYTES {
        return Err(format!("{path} is too large to outline ({} KB, max 2048 KB)", meta.len() / 1024));
    }
    let bytes = fs::read(&file).map_err(|e| format!("cannot read {path}: {e}"))?;
    if looks_binary(&bytes) {
        return Err(format!("{path} looks like a binary file"));
    }
    let text = String::from_utf8_lossy(&bytes);
    let Some(lang) = lang_of(&file) else {
        let ext = file.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
        let what = if ext.is_empty() { "files without an extension".to_string() } else { format!("{ext} files") };
        let mut out = format!("no outline for {what}; showing the first 40 lines:");
        for l in text.lines().map(str::trim).filter(|l| !l.is_empty()).take(40) {
            out.push('\n');
            out.push_str(&cut(l, SIG_MAX));
        }
        return Ok(out);
    };

    let defs = parse(lang, &text);
    let limit = limit.max(1);
    let mut out = format!("{} — {} lines, {}", rel_display(&root_c, &file), text.lines().count(), label(&file, lang));
    if defs.is_empty() {
        out.push_str("\n(no definitions found)");
        return Ok(out);
    }
    for d in defs.iter().take(limit) {
        out.push_str(&format!("\n{}L{} {}", "  ".repeat(d.level), d.line, d.sig));
    }
    if defs.len() > limit {
        out.push_str(&format!("\n(+{} more definitions)", defs.len() - limit));
    }
    Ok(out)
}

/// Whether a definition named `def` answers a query for `query` (`Foo::bar` also matches `bar`).
fn name_matches(def: &str, query: &str) -> bool {
    def == query
        || def.rsplit("::").next() == Some(query)
        || (query.contains("::") && query.rsplit("::").next() == Some(def))
}

fn skip_file_name(name: &str) -> bool {
    name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".map")
        || name.ends_with(".lock")
        || name == "package-lock.json"
}

struct FindState {
    scanned: usize,
    capped: bool,
    hits: Vec<(String, usize, String)>,
}

/// Depth-first walk in sorted order (deterministic), stopping at the file cap or `limit` hits.
fn walk(dir: &Path, root: &Path, name: &str, limit: usize, st: &mut FindState) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if st.capped || st.hits.len() >= limit {
            return;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let fname = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if ft.is_dir() {
            if fname.starts_with('.') || SKIP_DIRS.contains(&fname.as_str()) {
                continue;
            }
            walk(&path, root, name, limit, st);
            continue;
        }
        if !ft.is_file() || skip_file_name(&fname) {
            continue;
        }
        // JSON is skipped: its "definitions" are just keys, which is noise for symbol search.
        let Some(lang) = lang_of(&path).filter(|&l| l != Lang::Json) else { continue };
        let Ok(meta) = e.metadata() else { continue };
        if meta.len() > MAX_FIND_BYTES {
            continue;
        }
        if st.scanned >= MAX_SCANNED_FILES {
            st.capped = true;
            return;
        }
        st.scanned += 1;
        let Ok(bytes) = fs::read(&path) else { continue };
        if looks_binary(&bytes) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let rel = rel_display(root, &path);
        for d in parse(lang, &text) {
            if name_matches(&d.name, name) {
                st.hits.push((rel.clone(), d.line, d.sig));
                if st.hits.len() >= limit {
                    return;
                }
            }
        }
    }
}

/// Where is the identifier `name` defined? One `path:L<line> <signature>` line per definition.
pub fn find(root: &Path, name: &str, limit: usize) -> Result<String, String> {
    let name = name.trim();
    let valid = !name.is_empty()
        && name.chars().count() <= 100
        && name.chars().all(|c| is_ident_char(c) || c == '-' || c == ':');
    if !valid {
        return Err("name must be a plain identifier (letters, digits, _, $, -, ::; at most 100 characters)".into());
    }
    let mut st = FindState { scanned: 0, capped: false, hits: Vec::new() };
    walk(root, root, name, limit.max(1), &mut st);
    let note = format!("search capped at {MAX_SCANNED_FILES} files");
    if st.hits.is_empty() {
        let mut out = format!("no definition of \"{name}\" found");
        if st.capped {
            out.push_str(&format!(" ({note})"));
        }
        return Ok(out);
    }
    st.hits.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    st.hits.truncate(limit.max(1));
    let mut out = st.hits.iter().map(|(p, l, s)| format!("{p}:L{l} {s}")).collect::<Vec<_>>().join("\n");
    if st.capped {
        out.push_str(&format!("\n({note})"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A fresh, unique temp directory (removed by `Drop`).
    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir().join(format!("astralis-code-{}-{n}-{nanos}", std::process::id()));
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn write(&self, rel: &str, content: &[u8]) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn rust_outline_nests_methods_and_skips_comments() {
        let t = Tmp::new();
        t.write(
            "a.rs",
            b"use std::fmt;\n\n/// doc\npub struct Foo { a: i32 }\n\nimpl Foo {\n    pub fn new() -> Self { Foo { a: 1 } }\n    fn helper(&self, x: i32) -> i32 {\n        x\n    }\n}\n\n#[tauri::command]\npub async fn cmd(a: i32) {}\n\npub(crate) const MAX: usize = 3;\n// fn commented() {}\n/*\nfn in_block() {}\n*/\n",
        );
        let out = outline(&t.0, "a.rs", 50).unwrap();
        assert!(out.starts_with("a.rs — 20 lines, Rust"), "{out}");
        assert!(out.contains("\nL4 pub struct Foo"), "{out}");
        assert!(out.contains("\nL6 impl Foo"), "{out}");
        assert!(out.contains("\n  L7 pub fn new() -> Self"), "{out}");
        assert!(out.contains("\n  L8 fn helper(&self, x: i32) -> i32"), "{out}");
        assert!(out.contains("\nL14 pub async fn cmd(a: i32)"), "{out}");
        assert!(out.contains("pub(crate) const MAX: usize = 3;"), "{out}");
        assert!(!out.contains("commented") && !out.contains("in_block") && !out.contains("tauri"), "{out}");
    }

    #[test]
    fn typescript_outline() {
        let t = Tmp::new();
        t.write(
            "src/a.ts",
            b"import x from 'y';\nexport function foo(a: number) {\n  return a;\n}\nexport class Bar extends Baz {\n  private count = 0;\n  async run(x: string): Promise<void> {\n    if (x) {\n      this.go();\n    }\n  }\n}\nexport interface Props { a: string }\nexport const Comp = (props: Props) => {\n  return null;\n};\nconst local = 5;\n",
        );
        let out = outline(&t.0, "src/a.ts", 50).unwrap();
        assert!(out.starts_with("src/a.ts — 17 lines, TypeScript"), "{out}");
        assert!(out.contains("\nL2 export function foo(a: number)"), "{out}");
        assert!(out.contains("\nL5 export class Bar extends Baz"), "{out}");
        assert!(out.contains("\n  L7 async run(x: string): Promise<void>"), "{out}");
        assert!(out.contains("\nL13 export interface Props"), "{out}");
        assert!(out.contains("\nL14 export const Comp = (props: Props) =>"), "{out}");
        assert!(!out.contains("if (x)") && !out.contains("local") && !out.contains("count"), "{out}");
    }

    #[test]
    fn python_outline_skips_docstrings() {
        let t = Tmp::new();
        t.write(
            "m.py",
            b"import os\n\nclass Foo:\n    \"\"\"Doc\n    def fake(self): pass\n    \"\"\"\n    def method(self, x):\n        return x\n\nasync def run():\n    pass\n",
        );
        let out = outline(&t.0, "m.py", 50).unwrap();
        assert!(out.contains("\nL3 class Foo:"), "{out}");
        assert!(out.contains("\n  L7 def method(self, x):"), "{out}");
        assert!(out.contains("\nL10 async def run():"), "{out}");
        assert!(!out.contains("fake"), "{out}");
    }

    #[test]
    fn markdown_headings_are_indented_by_level() {
        let t = Tmp::new();
        t.write("r.md", b"# Title\n\ntext\n\n## Sub\n```\n# not a heading\n```\n### Deep\n");
        let out = outline(&t.0, "r.md", 50).unwrap();
        assert!(out.contains("\nL1 # Title"), "{out}");
        assert!(out.contains("\n  L5 ## Sub"), "{out}");
        assert!(out.contains("\n    L9 ### Deep"), "{out}");
        assert!(!out.contains("not a heading"), "{out}");
    }

    #[test]
    fn other_languages_smoke() {
        let t = Tmp::new();
        t.write("a.go", b"package a\n\ntype Server struct {\n}\n\nfunc (s *Server) Start() error {\n\treturn nil\n}\n");
        t.write("A.java", b"public class A {\n    @Override\n    public void run(int x) {\n        foo(x);\n        return bar(x);\n    }\n}\n");
        t.write("c.c", b"#include <x.h>\nstatic int add(int a, int b) {\n    return add2(a, b);\n}\nstruct point {\n    int x;\n};\n");
        t.write("Cargo.toml", b"[package]\nname = \"x\"\n\n[dependencies]\n");
        t.write("q.sql", b"-- c\nCREATE TABLE IF NOT EXISTS notes (\n  id int\n);\n");
        t.write("s.sh", b"#!/bin/sh\nbuild() {\n  echo hi\n}\nfunction deploy {\n  true\n}\n");
        assert!(outline(&t.0, "a.go", 9).unwrap().contains("L6 func (s *Server) Start() error"));
        assert!(outline(&t.0, "a.go", 9).unwrap().contains("L3 type Server struct"));
        let j = outline(&t.0, "A.java", 9).unwrap();
        assert!(j.contains("L1 public class A") && j.contains("  L3 public void run(int x)"), "{j}");
        assert!(!j.contains("foo(x)") && !j.contains("return"), "{j}");
        let c = outline(&t.0, "c.c", 9).unwrap();
        assert!(c.contains("L2 static int add(int a, int b)") && c.contains("L5 struct point"), "{c}");
        assert!(!c.contains("add2"), "{c}");
        let toml = outline(&t.0, "Cargo.toml", 9).unwrap();
        assert!(toml.contains("L1 [package]") && toml.contains("L4 [dependencies]"), "{toml}");
        assert!(outline(&t.0, "q.sql", 9).unwrap().contains("L2 CREATE TABLE IF NOT EXISTS notes"));
        let sh = outline(&t.0, "s.sh", 9).unwrap();
        assert!(sh.contains("L2 build() {") || sh.contains("L2 build()"), "{sh}");
        assert!(sh.contains("L5 function deploy"), "{sh}");
    }

    #[test]
    fn unknown_extension_shows_first_lines() {
        let t = Tmp::new();
        t.write("x.xyz", b"\n  one  \n\ntwo\n");
        let out = outline(&t.0, "x.xyz", 10).unwrap();
        assert_eq!(out, "no outline for .xyz files; showing the first 40 lines:\none\ntwo");
    }

    #[test]
    fn refuses_paths_outside_the_project() {
        let t = Tmp::new();
        t.write("proj/ok.rs", b"fn a() {}\n");
        t.write("secret.rs", b"fn s() {}\n");
        let root = t.0.join("proj");
        assert_eq!(outline(&root, "../x", 10).unwrap_err(), "path is outside the project");
        assert_eq!(outline(&root, "../secret.rs", 10).unwrap_err(), "path is outside the project");
        let abs = t.0.join("secret.rs");
        assert!(outline(&root, abs.to_str().unwrap(), 10).unwrap_err().contains("outside"));
        // An absolute path inside the project is fine.
        let inside = root.join("ok.rs");
        assert!(outline(&root, inside.to_str().unwrap(), 10).unwrap().contains("L1 fn a()"));
        assert!(outline(&root, "missing.rs", 10).unwrap_err().contains("not found"));
    }

    #[test]
    fn refuses_directories_binaries_and_big_files() {
        let t = Tmp::new();
        t.write("d/x.rs", b"fn a() {}\n");
        t.write("bin.rs", &[b'f', 0, 1, 2]);
        t.write("big.rs", &vec![b'a'; 2 * 1024 * 1024 + 1]);
        assert!(outline(&t.0, "d", 10).unwrap_err().contains("directory"));
        assert!(outline(&t.0, "bin.rs", 10).unwrap_err().contains("binary"));
        assert!(outline(&t.0, "big.rs", 10).unwrap_err().contains("too large"));
    }

    #[test]
    fn limit_caps_the_outline() {
        let t = Tmp::new();
        let src: String = (0..10).map(|i| format!("fn f{i}() {{}}\n")).collect();
        t.write("many.rs", src.as_bytes());
        let out = outline(&t.0, "many.rs", 3).unwrap();
        assert_eq!(out.lines().count(), 5, "{out}");
        assert!(out.ends_with("(+7 more definitions)"), "{out}");
    }

    #[test]
    fn find_locates_symbols_and_skips_node_modules() {
        let t = Tmp::new();
        t.write("src/deep/mod.rs", b"// nothing\npub fn target_fn() -> u8 { 1 }\n");
        t.write("node_modules/x/index.js", b"function target_fn() {}\n");
        t.write(".git/hooks/x.rs", b"fn target_fn() {}\n");
        t.write("web/a.ts", b"export const target_fn = () => 1;\n");
        let out = find(&t.0, "target_fn", 10).unwrap();
        assert_eq!(
            out,
            "src/deep/mod.rs:L2 pub fn target_fn() -> u8\nweb/a.ts:L1 export const target_fn = () => 1;"
        );
        assert_eq!(find(&t.0, "target_fn", 1).unwrap().lines().count(), 1);
        assert_eq!(find(&t.0, "nope", 10).unwrap(), "no definition of \"nope\" found");
        // Whole-word: a prefix of the name doesn't match.
        assert_eq!(find(&t.0, "target", 10).unwrap(), "no definition of \"target\" found");
    }

    #[test]
    fn find_rejects_bad_names() {
        let t = Tmp::new();
        assert!(find(&t.0, "", 5).is_err());
        assert!(find(&t.0, "a b", 5).is_err());
        assert!(find(&t.0, "foo(", 5).is_err());
        assert!(find(&t.0, &"a".repeat(101), 5).is_err());
        assert!(find(&t.0, "Foo::bar", 5).is_ok());
    }
}
