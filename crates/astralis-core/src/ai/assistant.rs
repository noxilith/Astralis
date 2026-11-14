//! "Ask Astralis": the in-app assistant for the note you're writing. It does one kind of work:
//! rewrite, expand, shorten, fix, format, continue or summarize a note (or the selected part of
//! it), or answer a question about it. The model itself is in `llm.rs`; this part is plain text
//! and testable without it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: ChatRole,
    pub text: String,
}

/// Everything the model sees for one reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub system: String,
    pub turns: Vec<ChatTurn>,
}

/// What a reply is for, which decides what the app offers to do with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    /// The new version of the text (the note, or the selection): can replace it.
    Rewrite,
    /// New text to add after the note or the selection.
    Insert,
    /// An answer to read (or copy).
    Answer,
}

/// A one-click job in the panel.
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub group: &'static str,
    pub output: Output,
    instruction: &'static str,
}

pub const ACTIONS: &[Action] = &[
    Action { id: "improve", label: "Improve writing", hint: "Clearer, better flow", group: "Rewrite", output: Output::Rewrite,
        instruction: "Rewrite the text so it reads clearly and flows well. Keep its meaning, structure and roughly its length. Fix any mistakes." },
    Action { id: "expand", label: "Expand", hint: "More detail, same voice", group: "Rewrite", output: Output::Rewrite,
        instruction: "Expand the text: develop each point with more detail and examples taken from what it already says. Keep its structure and voice. Don't invent facts; if a detail isn't there, leave it out. Make it about one and a half to two times longer." },
    Action { id: "shorten", label: "Shorten", hint: "Half as long", group: "Rewrite", output: Output::Rewrite,
        instruction: "Make the text about half as long. Keep every key point; cut repetition and filler." },
    Action { id: "grammar", label: "Fix grammar", hint: "Spelling and punctuation only", group: "Rewrite", output: Output::Rewrite,
        instruction: "Fix only spelling, grammar and punctuation. Don't reword, reorder, add or remove anything else." },
    Action { id: "format", label: "Make it look better", hint: "Headings, lists, emphasis", group: "Rewrite", output: Output::Rewrite,
        instruction: "Format the text so it looks good and is easy to scan: a clear # title if it has none, ## headings for sections, short paragraphs, - bullet lists where it lists things, and **bold** for key terms. Don't change the wording and don't add content." },
    Action { id: "professional", label: "Professional tone", hint: "Polished and formal", group: "Rewrite", output: Output::Rewrite,
        instruction: "Rewrite the text in a professional, polished tone. Keep the meaning and the facts." },
    Action { id: "casual", label: "Casual tone", hint: "Relaxed and friendly", group: "Rewrite", output: Output::Rewrite,
        instruction: "Rewrite the text in a relaxed, friendly, conversational tone. Keep the meaning and the facts." },
    Action { id: "checklist", label: "Make a checklist", hint: "Tasks you can tick off", group: "Create", output: Output::Rewrite,
        instruction: "Turn the text into a checklist: one `- [ ]` line per task or step, short and starting with a verb, in the same order. Use ## headings to group them if there are many." },
    Action { id: "continue", label: "Continue writing", hint: "The next paragraphs", group: "Create", output: Output::Insert,
        instruction: "Continue writing from where the text ends: add the next one to three paragraphs (or the next list items) in the same style and voice. Reply with ONLY the new text, not the existing text." },
    Action { id: "summarize", label: "Summarize", hint: "Key points at a glance", group: "Create", output: Output::Insert,
        instruction: "Summarize the text: one sentence, then 3 to 5 bullet points with the key points. Reply with ONLY the summary." },
    Action { id: "titles", label: "Suggest titles", hint: "Five options", group: "Ask", output: Output::Answer,
        instruction: "Suggest 5 short, specific titles for this note as a numbered list. Nothing else." },
    Action { id: "feedback", label: "Feedback", hint: "What to fix or add", group: "Ask", output: Output::Answer,
        instruction: "Give honest, specific feedback on the writing: what's unclear, what's missing, what to cut or reorder. 3 to 6 short bullet points. Don't rewrite the text." },
];

pub fn action(id: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|a| a.id == id)
}

/// What the user asked for.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// One of [`ACTIONS`] (by id), or none for a message typed in the box.
    pub action: Option<String>,
    /// What was typed (extra wishes for an action, or the whole request).
    pub message: String,
    /// The part of the note that's selected; the work applies to it instead of the whole note.
    pub selection: Option<String>,
}

/// The kind of reply a typed message is after: a question gets an answer; anything else is a
/// request to change the text.
pub fn output_for(message: &str) -> Output {
    let m = message.trim().to_lowercase();
    let first = m.split_whitespace().next().unwrap_or("");
    let question = m.ends_with('?')
        || matches!(first, "what" | "what's" | "whats" | "why" | "how" | "who" | "when" | "where" | "which" | "is" | "are" | "does" | "do" | "did" | "can" | "could" | "should" | "would" | "will" | "any" | "explain" | "tell" | "list" | "suggest" | "give" | "find");
    let write = matches!(first, "continue" | "write" | "add" | "append") && !m.starts_with("add to ") && !m.contains("section");
    if question {
        Output::Answer
    } else if write && (m.starts_with("continue") || m.contains("paragraph") || m.contains("next")) {
        Output::Insert
    } else {
        Output::Rewrite
    }
}

/// Who the assistant is. Kept short: a small model follows a few firm rules better than many.
pub const PERSONA: &str = "You are the writing assistant inside the user's notes app. You work only on the user's note.

Rules:
- Keep the user's language, voice, names and facts. Never invent facts, numbers, dates, names or quotes.
- Keep Markdown that is already there: # headings, - lists, - [ ] tasks, **bold**, [[links]] and images. Never delete a [[link]] or an image.
- When you write or edit, reply with ONLY the resulting text: no introduction, no explanation of what you changed, no closing remark and no code block.
- When asked a question, answer it briefly and specifically from the note. If the note doesn't say, say so.";

/// The longest text a rewrite can take: the whole thing has to fit in the reply as well.
pub const REWRITE_MAX: usize = 3200;

const FORMAT_RULE: &str = "Reply with ONLY the resulting text.";

/// Tidies a finished reply. The small model sometimes wraps the whole answer in a code block, or
/// repeats it after a line like "Written Markdown:"; both are cut away.
pub fn clean_reply(text: &str) -> String {
    let mut t = text.trim();
    // A repeat after "Written Markdown:" (or "Markdown:") on its own line.
    if let Some(i) = t.lines().position(|l| {
        let l = l.trim().trim_matches('*').to_lowercase();
        l.ends_with("markdown:") && l.len() < 30
    }) {
        let cut: usize = t.lines().take(i).map(|l| l.len() + 1).sum();
        let head = t[..cut.min(t.len())].trim();
        if !head.is_empty() {
            t = head;
        }
    }
    // An introduction line ("Sure! Here is the improved note:") before the text.
    if let Some((first, rest)) = t.split_once('\n') {
        let f = first.trim().to_lowercase();
        if f.ends_with(':') && f.len() < 80 && ["sure", "here", "certainly", "okay", "of course", "absolutely"].iter().any(|w| f.starts_with(w)) && !rest.trim().is_empty() {
            t = rest.trim();
        }
    }
    // The note's quoting echoed back: around everything, or a copy of the note after the answer.
    if let Some(rest) = t.strip_prefix("<note>").or_else(|| t.strip_prefix("\"\"\"")) {
        t = rest.trim();
        t = t.strip_suffix("</note>").or_else(|| t.strip_suffix("\"\"\"")).unwrap_or(t).trim();
    }
    if let Some(i) = [t.find("\n<note>"), t.find("\n\"\"\"")].into_iter().flatten().min() {
        t = t[..i].trim();
    }
    // A code block around everything.
    if let Some(rest) = t.strip_prefix("```") {
        let body = rest.split_once('\n').map_or("", |(_, b)| b);
        t = body.trim_end().strip_suffix("```").unwrap_or(body).trim();
    }
    // Formatting marks left dangling at the end by a cut (a lone "**", "#" or "-").
    loop {
        let trimmed = t.trim_end();
        let last = trimmed.lines().last().unwrap_or("").trim();
        if !last.is_empty() && last.chars().all(|c| matches!(c, '*' | '#' | '-' | '_' | '>' | '`')) {
            t = trimmed[..trimmed.len() - last.len()].trim_end();
        } else {
            break t.trim().to_string();
        }
    }
}

/// Keeps the note within `max_chars` (head and tail, which usually hold the point and the latest).
pub fn trim_note(body: &str, max_chars: usize) -> String {
    if body.len() <= max_chars {
        return body.to_string();
    }
    let head_len = max_chars * 2 / 3;
    let tail_len = max_chars - head_len;
    let mut h = head_len;
    while !body.is_char_boundary(h) {
        h -= 1;
    }
    let mut t = body.len() - tail_len;
    while !body.is_char_boundary(t) {
        t += 1;
    }
    format!("{}\n\n[… middle of the note left out …]\n\n{}", &body[..h], &body[t..])
}

/// Puts a request together: the persona, the text to work on, a little conversation and the job.
/// `budget_chars` bounds the whole thing (the model has a small context window). Fails, with a
/// message for the user, when a rewrite wouldn't fit (rewriting a cut-down note would lose text).
pub fn build_prompt(note_title: &str, note_body: &str, history: &[ChatTurn], req: &Request, budget_chars: usize) -> Result<(Prompt, Output), String> {
    let chosen = req.action.as_deref().map(|id| action(id).ok_or_else(|| format!("unknown action {id}"))).transpose()?;
    let output = chosen.map_or_else(|| output_for(&req.message), |a| a.output);
    let selection = req.selection.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let whole = note_body.trim();
    let text = selection.unwrap_or(whole);
    if text.is_empty() && output != Output::Answer && chosen.is_none_or(|a| a.id != "titles") {
        return Err("The note is empty. Write a little first, then ask me to work on it.".into());
    }
    if output == Output::Rewrite && text.chars().count() > REWRITE_MAX {
        return Err(if selection.is_some() {
            "That selection is too long for me to rewrite in one go. Select a shorter part (a few paragraphs).".into()
        } else {
            "This note is too long for me to rewrite in one go. Select a section (a few paragraphs) and ask again.".into()
        });
    }
    let shown = trim_note(text, budget_chars.saturating_sub(PERSONA.len() + 1200).clamp(1500, budget_chars / 2));
    let what = if selection.is_some() { "the selected text" } else { "the note" };
    let job = match chosen {
        Some(a) => {
            let extra = req.message.trim();
            if extra.is_empty() { a.instruction.to_string() } else { format!("{} Also: {extra}", a.instruction) }
        }
        None => match output {
            Output::Rewrite => format!("{} Apply this to the text and reply with the complete new text: {}", FORMAT_RULE, req.message.trim()),
            Output::Insert => format!("{} Write new text to go after the text, as asked: {}", FORMAT_RULE, req.message.trim()),
            Output::Answer => format!("Answer this about the text, briefly and specifically: {}", req.message.trim()),
        },
    };
    let context = if shown.is_empty() {
        format!("The note \"{}\" is empty so far.", note_title.trim())
    } else if selection.is_some() {
        format!("This is part of my note \"{}\". The selected text:\n\n\"\"\"\n{}\n\"\"\"", note_title.trim(), shown)
    } else {
        format!("My note is titled \"{}\". Its text:\n\n\"\"\"\n{}\n\"\"\"", note_title.trim(), shown)
    };
    let message = format!("{job}\n(Work on {what}.)");

    // Typed follow-ups can lean on the last exchange ("shorter", "now in French"); one-click
    // actions always start from the text as it is now.
    let mut left = budget_chars.saturating_sub(context.len() + message.len() + PERSONA.len());
    let mut kept: Vec<ChatTurn> = Vec::new();
    if chosen.is_none() {
        for t in history.iter().rev().take(2) {
            if t.text.len() > left / 2 {
                break;
            }
            left -= t.text.len();
            kept.push(t.clone());
        }
        kept.reverse();
        if kept.first().is_some_and(|t| t.role == ChatRole::Assistant) {
            kept.remove(0); // a conversation must start with the user
        }
    }
    let mut turns = Vec::with_capacity(kept.len() + 1);
    if kept.is_empty() {
        turns.push(ChatTurn { role: ChatRole::User, text: format!("{context}\n\n{message}") });
    } else {
        let mut first = kept.remove(0);
        first.text = format!("{context}\n\n{}", first.text);
        turns.push(first);
        turns.extend(kept);
        turns.push(ChatTurn { role: ChatRole::User, text: message });
    }
    Ok((Prompt { system: PERSONA.to_string(), turns }, output))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(action: Option<&str>, message: &str, selection: Option<&str>) -> Request {
        Request { action: action.map(Into::into), message: message.into(), selection: selection.map(Into::into) }
    }

    #[test]
    fn an_action_works_on_the_note_and_says_what_it_gives_back() {
        let (p, out) = build_prompt("Trip", "pack bags then go to airport", &[], &req(Some("improve"), "", None), 8000).unwrap();
        assert_eq!(out, Output::Rewrite);
        assert_eq!(p.turns.len(), 1);
        assert!(p.turns[0].text.contains("pack bags then go to airport") && p.turns[0].text.contains("Rewrite the text so it reads clearly"));
        assert!(p.turns[0].text.ends_with("(Work on the note.)"));
        assert!(p.system.contains("ONLY the resulting text"));
        assert_eq!(build_prompt("T", "x", &[], &req(Some("continue"), "", None), 8000).unwrap().1, Output::Insert);
        assert_eq!(build_prompt("T", "x", &[], &req(Some("feedback"), "", None), 8000).unwrap().1, Output::Answer);
        assert!(build_prompt("T", "x", &[], &req(Some("nope"), "", None), 8000).is_err());
    }

    #[test]
    fn a_selection_is_what_gets_worked_on() {
        let (p, _) = build_prompt("Trip", "Intro text.\n\nThe middle part.\n\nOutro.", &[], &req(Some("shorten"), "", Some("The middle part.")), 8000).unwrap();
        let t = &p.turns[0].text;
        assert!(t.contains("The selected text") && t.contains("The middle part.") && !t.contains("Intro text."));
        assert!(t.ends_with("(Work on the selected text.)"));
    }

    #[test]
    fn extra_wishes_ride_along_with_an_action() {
        let (p, _) = build_prompt("T", "hello", &[], &req(Some("expand"), "focus on the budget", None), 8000).unwrap();
        assert!(p.turns[0].text.contains("Also: focus on the budget"));
    }

    #[test]
    fn typed_messages_are_questions_or_edits() {
        for q in ["what is this note about?", "is the second paragraph clear", "how many tasks are left", "list the open questions", "suggest a better opening"] {
            assert_eq!(output_for(q), Output::Answer, "{q}");
        }
        for q in ["make the intro shorter", "translate it to French", "add a section about pricing", "rewrite the last paragraph", "remove the second bullet"] {
            assert_eq!(output_for(q), Output::Rewrite, "{q}");
        }
        for q in ["continue the story", "write the next paragraph"] {
            assert_eq!(output_for(q), Output::Insert, "{q}");
        }
    }

    #[test]
    fn a_follow_up_leans_on_the_last_exchange_but_an_action_does_not() {
        let history = vec![
            ChatTurn { role: ChatRole::User, text: "Make it formal".into() },
            ChatTurn { role: ChatRole::Assistant, text: "Dear sir,".into() },
        ];
        let (p, _) = build_prompt("T", "hello", &history, &req(None, "now shorter", None), 8000).unwrap();
        assert_eq!(p.turns.len(), 3);
        assert!(p.turns[0].text.contains("Make it formal") && p.turns[2].role == ChatRole::User);
        let (p, _) = build_prompt("T", "hello", &history, &req(Some("shorten"), "", None), 8000).unwrap();
        assert_eq!(p.turns.len(), 1);
    }

    #[test]
    fn a_rewrite_that_would_lose_text_is_refused() {
        let long = "word ".repeat(1000);
        let err = build_prompt("Long", &long, &[], &req(Some("improve"), "", None), 8000).unwrap_err();
        assert!(err.contains("too long") && err.contains("Select"));
        // Reading or adding doesn't need the whole thing to fit.
        assert!(build_prompt("Long", &long, &[], &req(Some("summarize"), "", None), 8000).is_ok());
        assert!(build_prompt("Long", &long, &[], &req(None, "what is this about?", None), 8000).is_ok());
        // A short selection of a long note is fine.
        assert!(build_prompt("Long", &long, &[], &req(Some("improve"), "", Some("a short bit")), 8000).is_ok());
    }

    #[test]
    fn an_empty_note_asks_for_something_to_work_on() {
        assert!(build_prompt("New", "  ", &[], &req(Some("improve"), "", None), 8000).is_err());
        assert!(build_prompt("New", "  ", &[], &req(Some("titles"), "", None), 8000).is_ok());
    }

    #[test]
    fn long_notes_are_cut_to_fit_when_only_reading() {
        let body = "a".repeat(20_000);
        let (p, _) = build_prompt("Long", &body, &[], &req(Some("summarize"), "", None), 6000).unwrap();
        let total: usize = p.turns.iter().map(|t| t.text.len()).sum();
        assert!(total <= 6000, "{total}");
        assert!(p.turns[0].text.contains("middle of the note left out"));
    }

    #[test]
    fn replies_lose_wrapping_code_blocks_and_preambles() {
        assert_eq!(clean_reply("```markdown\n# Title\n\nText\n```"), "# Title\n\nText");
        assert_eq!(clean_reply("Use ```rust code``` here"), "Use ```rust code``` here");
        assert_eq!(clean_reply("  plain  "), "plain");
        assert_eq!(clean_reply("Text.\n\n**\n\"\"\"\nnote copy"), "Text.");
        assert_eq!(clean_reply("<note>\n# Game\nText\n</note>"), "# Game\nText");
        assert_eq!(clean_reply("Sure! Here is the improved note:\n\n# Trip\nGo."), "# Trip\nGo.");
        assert_eq!(clean_reply("Here's a shorter version:\n\nPack and go."), "Pack and go.");
        assert_eq!(clean_reply("Here is what matters\nit is long"), "Here is what matters\nit is long");
    }
}
