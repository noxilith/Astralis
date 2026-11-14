//! Try the note assistant from the terminal, on a sample note:
//! `cargo run --release --example ask_assistant -- <model dir> [action id | message]`
//! (an action id is one of improve, expand, shorten, grammar, format, professional, casual,
//! checklist, continue, summarize, titles, feedback; `all` runs every action). Set
//! ASTRALIS_NOTE to use your own text and ASTRALIS_SELECTION to work on part of it.

use std::io::Write;
use std::time::Instant;

use astralis_core::ai::assistant::{ACTIONS, Request, action, build_prompt, clean_reply};
use astralis_core::ai::llm::Llm;

const NOTE_TITLE: &str = "Trip to Lisbon";
const NOTE: &str = "going to lisbon in march with sam. flight is tuesday morning, need to book hotel still. \
want to see the old town and eat pasteis de nata, maybe a day trip to sintra. budget is about 900 euros total. dont forget passport";

fn main() -> astralis_core::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("usage: ask_assistant <model dir> [action id | all | message]");
    let ask = args.collect::<Vec<_>>().join(" ");
    let jobs: Vec<Request> = if ask.is_empty() || ask == "all" {
        ACTIONS.iter().map(|a| Request { action: Some(a.id.into()), ..Default::default() }).collect()
    } else if action(&ask).is_some() {
        vec![Request { action: Some(ask), ..Default::default() }]
    } else {
        vec![Request { message: ask, ..Default::default() }]
    };

    let t = Instant::now();
    let mut llm = Llm::load(std::path::Path::new(&dir))?;
    eprintln!("[loaded in {:.1}s]", t.elapsed().as_secs_f32());

    let note = std::env::var("ASTRALIS_NOTE").unwrap_or_else(|_| NOTE.to_string());
    let selection = std::env::var("ASTRALIS_SELECTION").ok();
    for mut job in jobs {
        job.selection = selection.clone();
        eprintln!("\n> {}{}\n", job.action.as_deref().unwrap_or(""), job.message);
        let (prompt, output) = match build_prompt(NOTE_TITLE, &note, &[], &job, 8000) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("(refused: {e})");
                continue;
            }
        };
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
        if reply != raw.trim() {
            println!("\n\n--- tidied to ---\n{reply}");
        }
        let tokens = llm.count_tokens(&reply);
        eprintln!("\n\n[{output:?} · first word after {:.1}s · {tokens} tokens in {secs:.1}s · {:.1} tokens/s]", first.unwrap_or(0.0), tokens as f32 / secs);
    }
    Ok(())
}
