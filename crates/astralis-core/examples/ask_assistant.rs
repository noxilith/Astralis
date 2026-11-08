//! Try "Ask Astralis" from the terminal, with the app's persona and prompts on a sample game-idea note:
//! `cargo run --release --example ask_assistant -- <model dir> [all | quick prompt label | message]`.

use std::io::Write;
use std::time::Instant;

use astralis_core::ai::assistant::{ChatRole, ChatTurn, QUICK_PROMPTS, build_prompt, clean_reply};
use astralis_core::ai::llm::Llm;

const NOTE_TITLE: &str = "Game idea";
const NOTE: &str = "Space farming game. You grow crops on tiny planets, each planet has different gravity so plants grow weird. \
Sell crops to other colonies. Maybe multiplayer. Want to make it in Godot, I've never shipped a game before. Would sell for $15 on Steam.";

fn main() -> astralis_core::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("usage: ask_assistant <model dir> [all | prompt]");
    let ask = args.collect::<Vec<_>>().join(" ");
    // "a || b || c" is one conversation (each reply is history for the next), like the app.
    let chat = ask.contains("||");
    let messages: Vec<String> = if chat {
        ask.split("||").map(|m| m.trim().to_string()).collect()
    } else if ask.is_empty() || ask == "all" {
        QUICK_PROMPTS.iter().map(|(_, said, _)| said.to_string()).collect()
    } else {
        vec![QUICK_PROMPTS.iter().find(|(label, _, _)| label.eq_ignore_ascii_case(&ask)).map_or(ask.clone(), |(_, said, _)| said.to_string())]
    };

    let t = Instant::now();
    let mut llm = Llm::load(std::path::Path::new(&dir))?;
    eprintln!("[loaded in {:.1}s]", t.elapsed().as_secs_f32());

    let mut history: Vec<ChatTurn> = Vec::new();
    for message in messages {
        eprintln!("\n> {message}\n");
        let note = std::env::var("ASTRALIS_NOTE").unwrap_or_else(|_| NOTE.to_string());
        let prompt = build_prompt(NOTE_TITLE, &note, if chat { &history } else { &[] }, &message, 8000);
        let t = Instant::now();
        let mut first: Option<f32> = None;
        let raw = llm.generate(&prompt, 1100, |piece| {
            first.get_or_insert(t.elapsed().as_secs_f32());
            print!("{piece}");
            let _ = std::io::stdout().flush();
            true
        })?;
        let secs = t.elapsed().as_secs_f32();
        let reply = clean_reply(&raw);
        history.push(ChatTurn { role: ChatRole::User, text: message.clone() });
        history.push(ChatTurn { role: ChatRole::Assistant, text: reply.clone() });
        if reply != raw.trim() {
            println!("\n\n--- tidied to ---\n{reply}");
        }
        let tokens = llm.count_tokens(&reply);
        eprintln!("\n\n[first word after {:.1}s · {tokens} tokens in {secs:.1}s · {:.1} tokens/s]", first.unwrap_or(0.0), tokens as f32 / secs);
    }
    Ok(())
}
