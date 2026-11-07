//! Grammar and spelling checks with Harper — a rule-based, offline checker (no AI model,
//! no network). Spans are returned as UTF-16 offsets so the frontend can use them directly.

use std::sync::Arc;

use harper_core::linting::{LintGroup, LintKind, Linter, Suggestion};
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};
use serde::{Deserialize, Serialize};

use crate::util::char_to_utf16_offsets;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrammarFix {
    /// "replace", "insert" or "remove".
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrammarIssue {
    /// UTF-16 offsets into the checked text.
    pub start: usize,
    pub end: usize,
    pub message: String,
    pub kind: String,
    pub fixes: Vec<GrammarFix>,
}

pub struct Grammar {
    dictionary: Arc<FstDictionary>,
    linter: LintGroup,
}

impl Default for Grammar {
    fn default() -> Self {
        Self::new()
    }
}

impl Grammar {
    /// Building the linter takes a moment; create once and reuse.
    pub fn new() -> Self {
        let dictionary = FstDictionary::curated();
        let linter = LintGroup::new_curated(dictionary.clone(), Dialect::American);
        Self { dictionary, linter }
    }

    /// Checks Markdown text (code blocks and link URLs are ignored).
    pub fn check(&mut self, text: &str) -> Vec<GrammarIssue> {
        let doc = Document::new_markdown_default(text, self.dictionary.as_ref());
        let mut lints = self.linter.lint(&doc);
        harper_core::remove_overlaps(&mut lints);
        let offsets = char_to_utf16_offsets(text);
        let at = |i: usize| {
            offsets
                .get(i)
                .copied()
                .unwrap_or(*offsets.last().unwrap_or(&0))
        };
        let mut out: Vec<GrammarIssue> = lints
            .into_iter()
            .map(|l| GrammarIssue {
                start: at(l.span.start),
                end: at(l.span.end),
                message: l.message,
                kind: l.lint_kind.to_string(),
                fixes: l
                    .suggestions
                    .into_iter()
                    .take(5)
                    .map(|s| match s {
                        Suggestion::ReplaceWith(c) => GrammarFix {
                            kind: "replace".into(),
                            text: c.into_iter().collect(),
                        },
                        Suggestion::InsertAfter(c) => GrammarFix {
                            kind: "insert".into(),
                            text: c.into_iter().collect(),
                        },
                        Suggestion::Remove => GrammarFix {
                            kind: "remove".into(),
                            text: String::new(),
                        },
                    })
                    .collect(),
            })
            .collect();
        out.sort_by_key(|i| i.start);
        out
    }

    /// Applies only unambiguous corrections: lints of a safe kind (spelling, typos, agreement,
    /// capitalization, punctuation, repeated words…) that have exactly one suggestion.
    /// Returns the corrected text and how many fixes were applied.
    pub fn apply_safe_fixes(&mut self, text: &str) -> (String, usize) {
        const SAFE: [LintKind; 8] = [
            LintKind::Spelling,
            LintKind::Typo,
            LintKind::Agreement,
            LintKind::Capitalization,
            LintKind::Punctuation,
            LintKind::Repetition,
            LintKind::BoundaryError,
            LintKind::Grammar,
        ];
        let doc = Document::new_markdown_default(text, self.dictionary.as_ref());
        let mut lints = self.linter.lint(&doc);
        harper_core::remove_overlaps(&mut lints);
        let mut chars: Vec<char> = text.chars().collect();
        let mut fixes: Vec<_> = lints
            .into_iter()
            .filter(|l| SAFE.contains(&l.lint_kind) && l.suggestions.len() == 1)
            .collect();
        fixes.sort_by_key(|l| std::cmp::Reverse(l.span.start));
        let mut applied = 0;
        for l in fixes {
            let (s, e) = (l.span.start.min(chars.len()), l.span.end.min(chars.len()));
            match &l.suggestions[0] {
                Suggestion::ReplaceWith(r) => {
                    chars.splice(s..e, r.iter().copied());
                }
                Suggestion::InsertAfter(r) => {
                    chars.splice(e..e, r.iter().copied());
                }
                Suggestion::Remove => {
                    chars.drain(s..e);
                }
            }
            applied += 1;
        }
        (chars.into_iter().collect(), applied)
    }
}
