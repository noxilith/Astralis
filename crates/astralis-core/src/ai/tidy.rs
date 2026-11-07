//! "Tidy up": makes a Markdown note clean and consistent, and — when it's one long unstructured
//! wall of text — organizes it into titled sections using the Brain's embeddings.
//! Never invents content; it only reformats and adds headings.

use serde::{Deserialize, Serialize};

use crate::ai::brain::Brain;
use crate::ai::embed::dot;
use crate::ai::text::{Corpus, key_terms, stem, strip_markdown, words};
use crate::error::Result;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TidyResult {
    pub body: String,
    /// Human-readable list of what changed (empty = already tidy).
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blank,
    Heading,
    List,
    Quote,
    Table,
    Rule,
    FenceOpen,
    Code,
    FenceClose,
    Para,
}

fn is_fence(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

fn is_rule(s: &str) -> bool {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3
        && (t.chars().all(|c| c == '-')
            || t.chars().all(|c| c == '*')
            || t.chars().all(|c| c == '_'))
}

/// Uppercases the first letter of plain prose ("buy milk" → "Buy milk"), leaving things like
/// `iPhone`, URLs, code, numbers and @mentions alone.
fn capitalize(text: &str) -> Option<String> {
    let first_word = text.split_whitespace().next()?;
    let mut chars = first_word.chars();
    let c = chars.next()?;
    if !c.is_ascii_lowercase() || first_word.len() < 2 {
        return None;
    }
    let rest_ok = first_word
        .chars()
        .all(|c| c.is_ascii_lowercase() || matches!(c, '\'' | ',' | '.' | ':' | ';' | '!' | '?'));
    if !rest_ok || text.contains('`') || first_word.contains("://") {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    out.push(c.to_ascii_uppercase());
    out.push_str(&text[1..]);
    Some(out)
}

/// Splits a list marker off a (left-trimmed) line: returns (marker kind, rest).
/// Kinds: "bullet", "task-open", "task-done", or a number for ordered items.
enum Marker<'a> {
    Bullet(&'a str),
    Task(bool, &'a str),
    Number(u64, &'a str),
}

fn parse_marker(t: &str) -> Option<Marker<'_>> {
    // Bare task boxes: "[ ] thing", "[x] thing", "[] thing"
    for (prefix, done) in [
        ("[ ] ", false),
        ("[] ", false),
        ("[x] ", true),
        ("[X] ", true),
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return Some(Marker::Task(done, rest));
        }
    }
    let mut chars = t.char_indices();
    let (_, c) = chars.next()?;
    if matches!(c, '-' | '*' | '+' | '•' | '–' | '·') {
        let after = &t[c.len_utf8()..];
        // "-[x] done" (checkbox without the space)
        for (prefix, done) in [
            ("[ ] ", false),
            ("[] ", false),
            ("[x] ", true),
            ("[X] ", true),
        ] {
            if let Some(r) = after.strip_prefix(prefix) {
                return Some(Marker::Task(done, r));
            }
        }
        let rest = if let Some(r) = after.strip_prefix(' ') {
            r
        } else if matches!(c, '•' | '·') {
            after
        } else {
            return None;
        };
        let rest = rest.trim_start();
        for (prefix, done) in [
            ("[ ] ", false),
            ("[] ", false),
            ("[x] ", true),
            ("[X] ", true),
        ] {
            if let Some(r) = rest.strip_prefix(prefix) {
                return Some(Marker::Task(done, r));
            }
        }
        return Some(Marker::Bullet(rest));
    }
    // "3. thing" / "3) thing"
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() && digits.len() <= 4 {
        let after = &t[digits.len()..];
        if let Some(rest) = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))
        {
            return Some(Marker::Number(digits.parse().ok()?, rest.trim_start()));
        }
    }
    None
}

/// Cleans up Markdown formatting. Returns the new text and a description of each kind of change.
pub fn format_markdown(text: &str) -> (String, Vec<String>) {
    let mut lines: Vec<(Kind, String)> = Vec::new();
    let mut in_code = false;
    let (mut headings, mut bullets, mut numbers, mut tasks, mut caps, mut trailing) =
        (0, 0, 0, 0, 0, 0);
    // Renumbering state for ordered lists: (indent, next number).
    let mut ordered: Option<(usize, u64)> = None;

    for raw in text.replace("\r\n", "\n").replace('\t', "  ").lines() {
        if is_fence(raw) {
            in_code = !in_code;
            let t = raw.trim_end().to_owned();
            lines.push((
                if in_code {
                    Kind::FenceOpen
                } else {
                    Kind::FenceClose
                },
                t,
            ));
            continue;
        }
        if in_code {
            lines.push((Kind::Code, raw.to_owned()));
            continue;
        }
        let line = raw.trim_end();
        if line.len() != raw.len() && !line.is_empty() {
            trailing += 1;
        }
        if line.is_empty() {
            lines.push((Kind::Blank, String::new()));
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let t = line.trim_start();
        let pad = " ".repeat(indent);

        if ordered.is_some_and(|(i, _)| indent < i || parse_marker(t).is_none() && indent == 0) {
            ordered = None;
        }

        // Headings: "##Title ##" → "## Title"
        if t.starts_with('#') {
            let level = t.chars().take_while(|&c| c == '#').count();
            let rest = &t[level..];
            let body = rest.trim().trim_end_matches('#').trim();
            // "#tag" at the start of a line is a hashtag, not a heading; "##Title" is a heading.
            if level <= 6 && !body.is_empty() && (level >= 2 || rest.starts_with(' ')) {
                let fixed = format!("{} {}", "#".repeat(level), body);
                if fixed != t {
                    headings += 1;
                }
                lines.push((Kind::Heading, fixed));
                continue;
            }
        }
        if is_rule(t) {
            lines.push((Kind::Rule, "---".into()));
            continue;
        }
        if t.starts_with('>') {
            lines.push((Kind::Quote, line.to_owned()));
            continue;
        }
        if t.starts_with('|') {
            lines.push((Kind::Table, line.to_owned()));
            continue;
        }
        match parse_marker(t) {
            Some(Marker::Bullet(rest)) => {
                let rest = capitalize(rest)
                    .inspect(|_| caps += 1)
                    .unwrap_or_else(|| rest.to_owned());
                let fixed = format!("{pad}- {rest}");
                if fixed != line {
                    bullets += 1;
                }
                lines.push((Kind::List, fixed));
            }
            Some(Marker::Task(done, rest)) => {
                let rest = capitalize(rest)
                    .inspect(|_| caps += 1)
                    .unwrap_or_else(|| rest.to_owned());
                let fixed = format!("{pad}- [{}] {rest}", if done { 'x' } else { ' ' });
                if fixed != line {
                    tasks += 1;
                }
                lines.push((Kind::List, fixed));
            }
            Some(Marker::Number(n, rest)) => {
                let num = match ordered {
                    Some((i, next)) if i == indent => next,
                    _ => n.max(1),
                };
                ordered = Some((indent, num + 1));
                let rest = capitalize(rest)
                    .inspect(|_| caps += 1)
                    .unwrap_or_else(|| rest.to_owned());
                let fixed = format!("{pad}{num}. {rest}");
                if fixed != line {
                    numbers += 1;
                }
                lines.push((Kind::List, fixed));
            }
            None => {
                // Indented text right under a list item is a continuation of that item.
                let prev_list = lines
                    .iter()
                    .rev()
                    .find(|(k, _)| *k != Kind::Blank)
                    .is_some_and(|(k, _)| *k == Kind::List);
                let prev_blank = lines.last().is_some_and(|(k, _)| *k == Kind::Blank);
                if indent > 0 && prev_list && !prev_blank {
                    lines.push((Kind::List, line.to_owned()));
                } else {
                    let starts_para = lines.last().is_none_or(|(k, _)| *k != Kind::Para);
                    let text = if starts_para {
                        capitalize(t).inspect(|_| caps += 1)
                    } else {
                        None
                    };
                    lines.push((
                        Kind::Para,
                        format!("{pad}{}", text.unwrap_or_else(|| t.to_owned())),
                    ));
                }
            }
        }
    }
    if in_code {
        lines.push((Kind::FenceClose, "```".into()));
    }

    // Spacing: one blank line between blocks, around headings, lists, tables and code.
    let mut out: Vec<String> = Vec::new();
    let mut prev: Option<Kind> = None;
    let mut blank_fixes = 0;
    let mut pending_blank = false;
    for (kind, text) in lines {
        if kind == Kind::Blank {
            if prev.is_some() {
                pending_blank = true;
            }
            continue;
        }
        if let Some(p) = prev {
            let block_change = matches!(kind, Kind::Heading | Kind::FenceOpen | Kind::Rule)
                || matches!(p, Kind::Heading | Kind::FenceClose | Kind::Rule)
                || ((p == Kind::List) != (kind == Kind::List)
                    && kind != Kind::Code
                    && p != Kind::FenceOpen)
                || ((p == Kind::Table) != (kind == Kind::Table))
                || ((p == Kind::Quote) != (kind == Kind::Quote));
            let inside_code = matches!(kind, Kind::Code | Kind::FenceClose) || p == Kind::FenceOpen;
            if !inside_code && (pending_blank || block_change) {
                if !pending_blank {
                    blank_fixes += 1;
                }
                out.push(String::new());
            } else if inside_code && pending_blank {
                out.push(String::new());
            }
        }
        pending_blank = false;
        out.push(text);
        prev = Some(kind);
    }
    let mut body = out.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }

    let mut changes = Vec::new();
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    if headings > 0 {
        changes.push(format!("Fixed {}", plural(headings, "heading", "headings")));
    }
    if bullets > 0 {
        changes.push(format!(
            "Made {} consistent",
            plural(bullets, "bullet point", "bullet points")
        ));
    }
    if numbers > 0 {
        changes.push(format!(
            "Renumbered {}",
            plural(numbers, "list item", "list items")
        ));
    }
    if tasks > 0 {
        changes.push(format!(
            "Formatted {}",
            plural(tasks, "checkbox", "checkboxes")
        ));
    }
    if caps > 0 {
        changes.push(format!("Capitalized {}", plural(caps, "line", "lines")));
    }
    let normalized_input = text.replace("\r\n", "\n");
    if blank_fixes > 0
        || (trailing > 0)
        || (body.trim_end() != normalized_input.trim_end() && changes.is_empty())
    {
        changes.push("Cleaned up spacing".into());
    }
    (body, changes)
}

fn has_headings(text: &str) -> bool {
    let mut in_code = false;
    for line in text.lines() {
        if is_fence(line) {
            in_code = !in_code;
        } else if !in_code && line.trim_start().starts_with('#') {
            let t = line.trim_start();
            let level = t.chars().take_while(|&c| c == '#').count();
            if level <= 6 && t[level..].starts_with(' ') {
                return true;
            }
        }
    }
    false
}

/// Splits text into blocks separated by blank lines (code blocks stay whole).
fn blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut in_code = false;
    for line in text.lines() {
        if is_fence(line) {
            in_code = !in_code;
        }
        if line.trim().is_empty() && !in_code {
            if !cur.is_empty() {
                out.push(cur.join("\n"));
                cur.clear();
            }
        } else {
            cur.push(line);
        }
    }
    if !cur.is_empty() {
        out.push(cur.join("\n"));
    }
    out
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Brain {
    /// If the text is a long run of paragraphs with no headings, splits it into topical
    /// sections (where the subject changes between neighbouring blocks) and titles each one
    /// with its key terms. Returns the new text and how many sections were added.
    pub fn organize_sections(&mut self, text: &str) -> Result<Option<(String, usize)>> {
        if has_headings(text) {
            return Ok(None);
        }
        let blocks = blocks(text);
        let prose: Vec<String> = blocks.iter().map(|b| strip_markdown(b)).collect();
        let word_count: usize = prose.iter().map(|p| words(p).len()).sum();
        if blocks.len() < 4 || word_count < 60 {
            return Ok(None);
        }
        let vectors = self.embed(&prose)?;
        let sims: Vec<f32> = vectors.windows(2).map(|w| dot(&w[0], &w[1])).collect();

        // Cut at the k-1 weakest links between neighbouring blocks (each section ≥ 2 blocks).
        let k = ((blocks.len() as f32 / 3.0).round() as usize).clamp(2, 6);
        let mut order: Vec<usize> = (0..sims.len()).collect();
        order.sort_by(|&a, &b| sims[a].total_cmp(&sims[b]));
        let mut cuts: Vec<usize> = Vec::new();
        for i in order {
            if cuts.len() + 1 >= k {
                break;
            }
            let boundary = i + 1; // section starts at block `boundary`
            let ok = boundary >= 2
                && blocks.len() - boundary >= 2
                && cuts.iter().all(|&c| c.abs_diff(boundary) >= 2);
            if ok {
                cuts.push(boundary);
            }
        }
        if cuts.is_empty() {
            return Ok(None);
        }
        cuts.sort_unstable();

        let mut bounds = vec![0];
        bounds.extend(&cuts);
        bounds.push(blocks.len());
        let sections: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
        let section_tokens: Vec<Vec<String>> = sections
            .iter()
            .map(|&(a, b)| {
                prose[a..b]
                    .iter()
                    .flat_map(|p| words(p))
                    .map(|w| stem(&w))
                    .collect()
            })
            .collect();
        let corpus = Corpus::new(section_tokens.iter().map(|t| t.as_slice()));

        let mut out = String::new();
        let mut used = std::collections::HashSet::new();
        for (n, &(a, b)) in sections.iter().enumerate() {
            let text = prose[a..b].join("\n");
            let terms = key_terms(&text, &corpus, 4);
            let picked: Vec<String> = terms
                .into_iter()
                .map(|(t, _)| t)
                .filter(|t| used.insert(t.clone()))
                .take(2)
                .collect();
            let heading = if picked.is_empty() {
                format!("Part {}", n + 1)
            } else {
                title_case(&picked.join(" & "))
            };
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("## {heading}\n\n"));
            out.push_str(&blocks[a..b].join("\n\n"));
            out.push('\n');
        }
        Ok(Some((out, sections.len())))
    }

    /// Full tidy: formatting, then (if it helps) organizing into sections.
    pub fn tidy(&mut self, text: &str) -> Result<TidyResult> {
        let (formatted, mut changes) = format_markdown(text);
        let body = match self.organize_sections(&formatted)? {
            Some((organized, n)) => {
                changes.push(format!("Organized into {n} titled sections"));
                format_markdown(&organized).0
            }
            None => formatted,
        };
        Ok(TidyResult { body, changes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_lists_headings_and_spacing() {
        let input = "##Goals##\n* ship it\n+ test it   \n\n\n\n3) first\n7) second\nsome text\n[ ] call bob\n-[x] Done thing\n```\n*  keep   \n```\n";
        let (out, changes) = format_markdown(input);
        assert_eq!(
            out,
            "## Goals\n\n- Ship it\n- Test it\n\n3. First\n4. Second\n\nSome text\n\n- [ ] Call bob\n- [x] Done thing\n\n```\n*  keep   \n```\n"
        );
        assert!(changes.iter().any(|c| c.contains("heading")));
        assert!(changes.iter().any(|c| c.contains("Renumbered")));
    }

    #[test]
    fn leaves_tidy_notes_alone() {
        let input = "# Title\n\nA paragraph with iPhone and `code`.\n\n- Item\n  - Nested\n\n1. One\n2. Two\n";
        let (out, changes) = format_markdown(input);
        assert_eq!(out, input);
        assert!(changes.is_empty(), "{changes:?}");
    }

    #[test]
    fn does_not_capitalize_code_or_brands() {
        assert_eq!(capitalize("npm install"), Some("Npm install".into()));
        assert_eq!(capitalize("iPhone notes"), None);
        assert_eq!(capitalize("`cargo` build"), None);
        assert_eq!(capitalize("https://x.com"), None);
    }

    #[test]
    fn organizes_unstructured_text_into_sections() {
        let mut brain = Brain::new(None);
        let text = "The sourdough starter needs flour and water every day. Feed the starter in the morning.\n\n\
            Knead the dough for ten minutes. Let the dough rise overnight in a warm kitchen.\n\n\
            Bake the bread in a very hot oven with steam for a crisp crust.\n\n\
            The quarterly budget review is on Friday. Finance wants the spreadsheet early.\n\n\
            Marketing spend went over budget in March. The budget needs cuts next quarter.\n\n\
            Send the budget spreadsheet to finance before the review meeting.\n";
        let result = brain.tidy(text).unwrap();
        assert!(
            result.changes.iter().any(|c| c.contains("sections")),
            "{result:?}"
        );
        let headings: Vec<&str> = result
            .body
            .lines()
            .filter(|l| l.starts_with("## "))
            .collect();
        assert_eq!(headings.len(), 2, "{}", result.body);
        // The split falls between the baking and the budget paragraphs.
        let second = result.body.find(headings[1]).unwrap();
        assert!(result.body[second..].contains("quarterly budget"));
        assert!(!result.body[second..].contains("sourdough"));
        // Already structured text isn't reorganized.
        assert!(brain.organize_sections(&result.body).unwrap().is_none());
    }
}
