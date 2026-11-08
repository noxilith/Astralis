//! "Ask Astralis": the in-app assistant's persona and how requests are put together. The model
//! itself is in `llm.rs`; this part is plain text and testable without it.

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

/// Who the assistant is. Kept short: a small model follows a few firm rules better than many.
pub const PERSONA: &str = "You are Astralis, the assistant inside the user's notes app. You are blunt, practical and brutally honest, like an experienced founder reviewing a pitch.

Rules:
- Never flatter. Don't call an idea good unless it really is. If it's a bad idea or won't pay off, say so plainly.
- Always be open about the return on investment: the realistic time and money it will cost against the realistic payoff (money, users or learning). Use the likely case, not the best case.
- Be specific to the user's note. Point out the real problems (scope, inexperience, crowded market, no clear hook, cost) instead of generic advice.
- When asked to write or rewrite the note, reply with ONLY the note itself (# headings, short paragraphs, - lists), with no preamble and no code block.
- Plans are concrete: phases, tasks, rough times, and the first step for today. Cut anything a first version doesn't need.
- Don't invent facts about the project; if something important is missing, say what.";

/// How every opinion is given: a verdict, what's wrong, the ROI and whether to do it.
const JUDGE: &str = "Answer in exactly this format:
**Verdict:** Good idea, Weak idea or Bad idea — then one blunt sentence why.
**What's wrong with it:** the biggest problems, specific to this note (and anything else I asked).
**Will it pay off?** Worth it, Not worth it, or Only if… — realistic cost (time and money, as ranges) against the realistic payoff (the likely case).
**Should you do it?** Yes, No, or Only if… — and the cheapest way to test it in a week or less.";

/// Writing the note: do what was asked, and still end with an honest take.
const WRITE: &str = "Reply with ONLY the note itself, doing what I asked: no preamble and no code block. End the note with a ## Honest take section: the verdict (Good, Weak or Bad idea) and whether it will pay off (realistic cost against realistic payoff), in 2 or 3 blunt lines.";

/// Planning: concrete, and still honest about whether it's worth it.
const PLAN: &str = "Answer in exactly this format:
**Scope for version 1:** what's in, and what's cut until later.
**Phases:** 3 to 5 phases, each with its tasks and a rough time.
**Milestones:** how they'll know each phase is done.
**First step today:** one task that takes under 2 hours.
**Honest take:** the verdict and whether it will pay off, in one or two blunt sentences.";

/// Coming up with ideas: a few honest options, each with its payoff, and a pick.
const IDEAS: &str = "Give me 3 to 5 ideas that fit what I asked (and my note, if it has anything). Use exactly this format for each:
### 1. Name — one sentence on what it is
**Why it could work:** who would want it and why.
**Competition:** the main apps or products it's up against, and how crowded it is.
**Will it pay off?** Worth it, Long shot, or Not worth it — rough effort against the realistic payoff.

After the ideas, end with **My pick:** the one with the best payoff (never one you rated Not worth it), why, and the first step to test it this week. Be honest: most ideas don't pay off, so don't call them all worth it. Prefer small, specific ideas one person can build and test over crowded ones.";

/// Any other question: a straight answer, still honest about whether it's worth it.
const ASK: &str = "Answer my question directly and specifically, in short paragraphs or a list. Be brutally honest. If it's about making or doing something, end with **Will it pay off?** and a one-line verdict on whether it's worth the time and money.";

/// Quick prompts offered in the panel: (label, what the user's message says, the answer format
/// the model is held to). The format is added only for the model, right before it replies.
pub const QUICK_PROMPTS: &[(&str, &str, &str)] = &[
    ("Be brutally honest", "Is this a good idea? Be brutally honest.", JUDGE),
    (
        "Will it pay off?",
        "Will this pay off?",
        "Answer in exactly this format:
**ROI verdict:** Worth it, Not worth it, or Only if… — one blunt sentence.
**What it will cost:** realistic time and money for this person, as ranges.
**Realistic payoff:** money, users or learning, as ranges, with the likely case (not the best case).
**Why:** the 2 or 3 facts that decide it.
**How to improve the odds:** the 2 changes that matter most.",
    ),
    ("Make a plan", "Make a plan to build and finish this.", PLAN),
    ("Give me ideas", "Give me some ideas worth building.", IDEAS),
    (
        "Write it up",
        "Rewrite this note so it explains the idea, lists its features and plans how to finish it.",
        "Reply with ONLY the new note itself: a # title, a short paragraph explaining the idea, a ## Features list, a ## Plan with phases and rough times, a ## Risks list, and a ## Honest take section with the verdict (Good, Weak or Bad idea) and whether it will pay off, in 2 or 3 blunt lines. No preamble and no code block.",
    ),
];

/// What a message asks for, which decides how the answer is shaped. Every kind stays honest
/// about whether it's worth it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Write or rewrite the note.
    Write,
    Plan,
    /// An opinion on the idea: verdict, problems, ROI.
    Judge,
    /// Come up with ideas.
    Ideas,
    /// Anything else: a straight answer.
    Ask,
}

pub fn intent(message: &str) -> Intent {
    let m = format!(" {} ", message.to_lowercase());
    let has = |words: &[&str]| words.iter().any(|w| m.contains(w));
    if has(&[
        " write", "rewrite", "re-write", "turn this", "turn it", "make this note", "make the note", "make it a note", "draft", "expand",
        "summarize", "summarise", "organize", "organise", "clean up", "clean this", "format this", "reword", "put this into", "fill this",
        "fill the note", "update the note", "add to the note", "into a note", "explain my", "list its", "list the features", "list out",
    ]) {
        Intent::Write
    } else if has(&[" plan", "roadmap", "steps to", "how do i build", "how do i make", "how should i build", "how should i start", "timeline", "milestone"]) {
        Intent::Plan
    } else if has(&[
        "good idea", "bad idea", "should i", "worth", "pay off", "payoff", " roi", "thoughts", "what do you think", "what if", "would people",
        "will people", "honest", "is this", "is it ", "rate ", "viable", "feedback", "opinion", "critique", "make this?", "do this?", "any good",
    ]) {
        Intent::Judge
    } else if has(&[
        "come up with", "give me", "ideas", "an idea", " idea?", "brainstorm", "suggest", "think of", "recommend", "inspire", "what should i make",
        "what should i build", "what can i make", "what could i make", "something to make", "something to build",
    ]) {
        Intent::Ideas
    } else {
        Intent::Ask
    }
}

/// The answer format kept right before the reply (a small model weighs what it read last most).
fn guide(message: &str) -> &'static str {
    if let Some((_, _, format)) = QUICK_PROMPTS.iter().find(|(_, said, _)| *said == message.trim()) {
        return format;
    }
    match intent(message) {
        Intent::Write => WRITE,
        Intent::Plan => PLAN,
        Intent::Judge => JUDGE,
        Intent::Ideas => IDEAS,
        Intent::Ask => ASK,
    }
}

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

/// Puts a request together: the persona, the note, the recent conversation and the new message.
/// `budget_chars` bounds the whole thing (the model has a small context window).
pub fn build_prompt(note_title: &str, note_body: &str, history: &[ChatTurn], message: &str, budget_chars: usize) -> Prompt {
    let note = trim_note(note_body.trim(), budget_chars / 2);
    let context = if note.is_empty() {
        format!("The note \"{}\" is empty so far.", note_title.trim())
    } else {
        format!("My note is titled \"{}\". This is what it says now:\n\n\"\"\"\n{}\n\"\"\"", note_title.trim(), note)
    };
    let message = format!("{}\n\n{}", message.trim(), guide(message));
    let message = message.as_str();
    // Keep the most recent turns that fit what's left.
    let mut left = budget_chars.saturating_sub(context.len() + message.len() + PERSONA.len());
    let mut kept: Vec<ChatTurn> = Vec::new();
    for t in history.iter().rev() {
        if t.text.len() > left {
            break;
        }
        left -= t.text.len();
        kept.push(t.clone());
    }
    kept.reverse();
    // The note goes with the first message the model sees.
    let mut turns = Vec::with_capacity(kept.len() + 1);
    if kept.first().is_some_and(|t| t.role == ChatRole::Assistant) {
        kept.remove(0); // a conversation must start with the user
    }
    if kept.is_empty() {
        turns.push(ChatTurn { role: ChatRole::User, text: format!("{context}\n\n{message}") });
    } else {
        let mut first = kept.remove(0);
        first.text = format!("{context}\n\n{}", first.text);
        turns.push(first);
        turns.extend(kept);
        turns.push(ChatTurn { role: ChatRole::User, text: message.to_string() });
    }
    Prompt { system: PERSONA.to_string(), turns }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_carries_the_note_and_recent_conversation() {
        let history = vec![
            ChatTurn { role: ChatRole::User, text: "I want to make a farming game.".into() },
            ChatTurn { role: ChatRole::Assistant, text: "What makes it different?".into() },
        ];
        let p = build_prompt("Game idea", "Cozy farming + space.", &history, "Be brutally honest.", 6000);
        assert!(p.system.contains("brutally honest"));
        assert_eq!(p.turns.len(), 3);
        assert!(p.turns[0].text.contains("Cozy farming + space.") && p.turns[0].text.contains("farming game"));
        assert_eq!(p.turns[2].role, ChatRole::User);
        assert!(p.turns[2].text.starts_with("Be brutally honest.") && p.turns[2].text.contains(JUDGE));
    }

    #[test]
    fn long_notes_and_histories_are_cut_to_fit() {
        let body = "a".repeat(20_000);
        let history: Vec<ChatTurn> = (0..50)
            .map(|i| ChatTurn { role: if i % 2 == 0 { ChatRole::User } else { ChatRole::Assistant }, text: "x".repeat(400) })
            .collect();
        let p = build_prompt("Long", &body, &history, "Summarize", 6000);
        let total: usize = p.turns.iter().map(|t| t.text.len()).sum();
        assert!(total <= 6000, "{total}");
        assert!(p.turns[0].text.contains("middle of the note left out"));
        assert_eq!(p.turns[0].role, ChatRole::User);
        assert!(p.turns.last().unwrap().text.starts_with("Summarize"));
    }

    #[test]
    fn quick_prompts_bring_their_answer_format() {
        let (_, said, format) = QUICK_PROMPTS[0];
        let p = build_prompt("Idea", "A thing.", &[], said, 6000);
        assert!(p.turns[0].text.ends_with(format));
    }

    #[test]
    fn every_kind_of_message_stays_honest_about_the_idea_and_its_roi() {
        for q in ["should i make this?", "is this a good idea", "thoughts on adding multiplayer?", "would people buy it"] {
            assert_eq!(intent(q), Intent::Judge, "{q}");
        }
        for q in ["make this note explain my idea for this game and list its features", "rewrite it shorter", "Write a pitch"] {
            assert_eq!(intent(q), Intent::Write, "{q}");
        }
        for q in ["make a plan to finish it", "what's the roadmap"] {
            assert_eq!(intent(q), Intent::Plan, "{q}");
        }
        for q in ["give me a good app idea", "can you help me come up with an idea", "brainstorm some side projects", "any ideas for a game?"] {
            assert_eq!(intent(q), Intent::Ideas, "{q}");
        }
        for q in ["which engine is easiest for 2d?", "how does steam pricing work"] {
            assert_eq!(intent(q), Intent::Ask, "{q}");
        }
        for format in [JUDGE, WRITE, PLAN, IDEAS, ASK] {
            assert!(format.contains("pay off"));
        }
        assert!(QUICK_PROMPTS.iter().all(|(_, _, f)| f.contains("pay off") || f.contains("ROI")));
    }

    #[test]
    fn replies_lose_wrapping_code_blocks_and_repeats() {
        assert_eq!(clean_reply("```markdown\n# Title\n\nText\n```"), "# Title\n\nText");
        assert_eq!(clean_reply("**Verdict:** Bad idea.\n Written Markdown:\n\n```markdown\n**Verdict:** Bad idea.\n```"), "**Verdict:** Bad idea.");
        assert_eq!(clean_reply("Use ```rust code``` here"), "Use ```rust code``` here");
        assert_eq!(clean_reply("  plain  "), "plain");
        assert_eq!(clean_reply("Text.\n\n**\n\"\"\"\nnote copy"), "Text.");
        assert_eq!(clean_reply("<note>\n# Game\nText\n</note>"), "# Game\nText");
        assert_eq!(clean_reply("**Verdict:** Weak idea.\n\n<note>\nGame idea: copy\n</note>"), "**Verdict:** Weak idea.");
    }

    #[test]
    fn an_empty_note_is_said_to_be_empty() {
        let p = build_prompt("New idea", "  ", &[], "Write it up", 6000);
        assert!(p.turns[0].text.starts_with("The note \"New idea\" is empty"));
    }
}
