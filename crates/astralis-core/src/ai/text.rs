//! Text utilities for the Brain: Markdown stripping, tokenizing, sentence splitting, TF-IDF.

use std::collections::{HashMap, HashSet};

const STOPWORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "after",
    "again",
    "against",
    "all",
    "also",
    "am",
    "an",
    "and",
    "any",
    "are",
    "aren't",
    "as",
    "at",
    "be",
    "because",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "by",
    "can",
    "cannot",
    "could",
    "did",
    "do",
    "does",
    "doing",
    "don't",
    "done",
    "down",
    "during",
    "each",
    "etc",
    "even",
    "ever",
    "every",
    "few",
    "for",
    "from",
    "further",
    "get",
    "gets",
    "got",
    "had",
    "has",
    "have",
    "having",
    "he",
    "her",
    "here",
    "hers",
    "herself",
    "him",
    "himself",
    "his",
    "how",
    "however",
    "i",
    "if",
    "in",
    "into",
    "is",
    "isn't",
    "it",
    "it's",
    "its",
    "itself",
    "just",
    "let",
    "like",
    "made",
    "make",
    "many",
    "may",
    "me",
    "might",
    "more",
    "most",
    "much",
    "must",
    "my",
    "myself",
    "need",
    "new",
    "no",
    "nor",
    "not",
    "now",
    "of",
    "off",
    "often",
    "on",
    "once",
    "one",
    "only",
    "or",
    "other",
    "our",
    "ours",
    "ourselves",
    "out",
    "over",
    "own",
    "per",
    "put",
    "quite",
    "rather",
    "really",
    "same",
    "see",
    "she",
    "should",
    "so",
    "some",
    "still",
    "such",
    "take",
    "than",
    "that",
    "the",
    "their",
    "theirs",
    "them",
    "themselves",
    "then",
    "there",
    "these",
    "they",
    "thing",
    "things",
    "this",
    "those",
    "through",
    "to",
    "too",
    "under",
    "until",
    "up",
    "upon",
    "us",
    "use",
    "used",
    "using",
    "very",
    "via",
    "want",
    "was",
    "way",
    "we",
    "well",
    "were",
    "what",
    "when",
    "where",
    "which",
    "while",
    "who",
    "whom",
    "why",
    "will",
    "with",
    "within",
    "without",
    "would",
    "yes",
    "yet",
    "you",
    "your",
    "yours",
    "yourself",
    "yourselves",
    "todo",
    "note",
    "notes",
    "untitled",
];

pub fn is_stopword(w: &str) -> bool {
    use std::sync::OnceLock;
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| STOPWORDS.iter().copied().collect())
        .contains(w)
}

/// Removes Markdown syntax but keeps the readable text. Code blocks are dropped.
pub fn strip_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let mut l = trimmed
            .trim_start_matches('#')
            .trim_start_matches('>')
            .trim_start();
        for marker in [
            "- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "- ", "* ", "+ ",
        ] {
            if let Some(rest) = l.strip_prefix(marker) {
                l = rest;
                break;
            }
        }
        out.push_str(&strip_inline(l));
        out.push('\n');
    }
    out
}

fn strip_inline(line: &str) -> String {
    let mut s = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // [[target|alias]] → alias (or target)
        if c == '['
            && chars.get(i + 1) == Some(&'[')
            && let Some(end) = find_seq(&chars, i + 2, &[']', ']'])
        {
            let inner: String = chars[i + 2..end].iter().collect();
            let shown = inner.rsplit('|').next().unwrap_or(&inner);
            s.push_str(shown.split('#').next().unwrap_or(shown));
            i = end + 2;
            continue;
        }
        // [text](url) → text ; ![alt](url) → alt
        if c == '[' || (c == '!' && chars.get(i + 1) == Some(&'[')) {
            let start = if c == '!' { i + 2 } else { i + 1 };
            if let Some(close) = find_seq(&chars, start, &[']', '('])
                && let Some(paren) = find_seq(&chars, close + 2, &[')'])
            {
                s.extend(&chars[start..close]);
                i = paren + 1;
                continue;
            }
        }
        if matches!(c, '*' | '_' | '`' | '~' | '=') {
            i += 1;
            continue;
        }
        s.push(c);
        i += 1;
    }
    s
}

fn find_seq(chars: &[char], from: usize, seq: &[char]) -> Option<usize> {
    (from..chars.len().saturating_sub(seq.len() - 1)).find(|&j| chars[j..j + seq.len()] == *seq)
}

/// Lowercased word tokens (letters/digits, internal `-` and `'` allowed).
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\''))
        .map(|w| w.trim_matches(|c| c == '-' || c == '\'').to_lowercase())
        .filter(|w| w.chars().count() >= 2 && !w.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

/// Words that carry meaning: no stopwords, at least 3 characters.
pub fn content_words(text: &str) -> Vec<String> {
    words(text)
        .into_iter()
        .filter(|w| w.chars().count() >= 3 && !is_stopword(w))
        .collect()
}

/// Very light normalization so "meetings"/"meeting" collapse together.
pub fn stem(w: &str) -> String {
    for suffix in ["ies", "es", "s"] {
        if w.len() > 4 && w.ends_with(suffix) && !w.ends_with("ss") {
            let base = &w[..w.len() - suffix.len()];
            return if suffix == "ies" {
                format!("{base}y")
            } else {
                base.to_owned()
            };
        }
    }
    w.to_owned()
}

/// Splits plain text into sentences. List items and lines without punctuation count as sentences.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut start = 0;
        for i in 0..chars.len() {
            let end_mark = matches!(chars[i], '.' | '!' | '?');
            let at_boundary = i + 1 == chars.len() || (chars[i + 1].is_whitespace() && end_mark);
            if end_mark && at_boundary {
                let s: String = chars[start..=i].iter().collect();
                push_sentence(&mut out, &s);
                start = i + 1;
            }
        }
        if start < chars.len() {
            let s: String = chars[start..].iter().collect();
            push_sentence(&mut out, &s);
        }
    }
    out
}

fn push_sentence(out: &mut Vec<String>, s: &str) {
    let s = s.trim();
    if words(s).len() >= 3 {
        out.push(s.to_owned());
    }
}

/// Document frequencies over a corpus of token sets.
pub struct Corpus {
    pub docs: usize,
    pub df: HashMap<String, usize>,
}

impl Corpus {
    pub fn new<'a>(docs: impl IntoIterator<Item = &'a [String]>) -> Self {
        let mut df: HashMap<String, usize> = HashMap::new();
        let mut n = 0;
        for doc in docs {
            n += 1;
            let unique: HashSet<&String> = doc.iter().collect();
            for t in unique {
                *df.entry(t.clone()).or_default() += 1;
            }
        }
        Self { docs: n, df }
    }

    pub fn idf(&self, term: &str) -> f32 {
        let df = self.df.get(term).copied().unwrap_or(0) as f32;
        ((1.0 + self.docs as f32) / (1.0 + df)).ln() + 1.0
    }
}

/// Candidate key terms (stemmed unigrams and bigrams) with TF-IDF scores, best first.
/// Returns `(display form, score)`; the display form is the most common surface spelling.
pub fn key_terms(text: &str, corpus: &Corpus, max: usize) -> Vec<(String, f32)> {
    let toks = words(text);
    let mut tf: HashMap<String, (f32, HashMap<String, usize>)> = HashMap::new();
    let mut bump = |key: String, surface: String, w: f32| {
        let e = tf.entry(key).or_insert_with(|| (0.0, HashMap::new()));
        e.0 += w;
        *e.1.entry(surface).or_default() += 1;
    };
    for (i, w) in toks.iter().enumerate() {
        if w.chars().count() >= 3 && !is_stopword(w) {
            bump(stem(w), w.clone(), 1.0);
            if let Some(next) = toks.get(i + 1)
                && next.chars().count() >= 3
                && !is_stopword(next)
            {
                bump(
                    format!("{} {}", stem(w), stem(next)),
                    format!("{w} {next}"),
                    1.0,
                );
            }
        }
    }
    let mut scored: Vec<(String, f32)> = tf
        .into_iter()
        .filter(|(_, (count, _))| *count > 0.0)
        .map(|(key, (count, surfaces))| {
            let is_bigram = key.contains(' ');
            if is_bigram && count < 2.0 {
                return (String::new(), 0.0);
            }
            let surface = surfaces
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                .map(|(s, _)| s)
                .unwrap_or_default();
            let idf = if is_bigram {
                key.split(' ').map(|p| corpus.idf(p)).sum::<f32>() / 2.0
            } else {
                corpus.idf(&key)
            };
            let boost = if is_bigram { 1.3 } else { 1.0 };
            (surface, (1.0 + count.ln()) * idf * boost)
        })
        .filter(|(s, score)| !s.is_empty() && *score > 0.0)
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scored.truncate(max);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_markdown() {
        let s = strip_markdown(
            "# Title\n- [ ] do **this** with [[Note A|alias]] and [link](http://x)\n```\ncode\n```\n",
        );
        assert_eq!(s, "Title\ndo this with alias and link\n");
    }

    #[test]
    fn splits_sentences() {
        let s =
            sentences("This is one sentence. And here is another! Short.\na list item with words");
        assert_eq!(
            s,
            vec![
                "This is one sentence.",
                "And here is another!",
                "a list item with words"
            ]
        );
    }

    #[test]
    fn stems() {
        assert_eq!(stem("meetings"), "meeting");
        assert_eq!(stem("libraries"), "library");
        assert_eq!(stem("class"), "class");
    }
}
