//! Fills the Brain with 1,000 test memories for trying the 3D brain graph at scale:
//! - 5 test projects of different sizes plus "Me",
//! - topics within each project whose memories share vocabulary, so related ones wire together,
//!   and some loners with no strong relations,
//! - every memory type, several AI tools as authors, new and reviewed, a few pinned,
//! - recall counts from never to very often (neuron size and firing),
//! - a code map per project.
//!
//! Usage: `cargo run -p astralis-core --example seed_test_brain -- <path to astralis.db>`
//! (or `./run.sh seed-test-brain`). Deterministic.

use astralis_core::Store;
use astralis_core::memory::{MapEntry, MemoryKind};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() % 10_000) as f64 / 10_000.0 < p
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

/// (project, share of the memories, topics: (name, vocabulary)).
type Topic = (&'static str, &'static [&'static str]);
const PROJECTS: &[(&str, f64, &[Topic])] = &[
    ("Astralis (test)", 0.34, &[
        ("sync", &["sync", "vault", "peer", "tombstone", "offer", "scope", "merge", "conflict", "clock", "share"]),
        ("editor", &["editor", "caret", "yjs", "crdt", "codemirror", "selection", "awareness", "keystroke", "undo", "markdown"]),
        ("graph", &["graph", "galaxy", "star", "canvas", "zoom", "pan", "nebula", "frame", "render", "orbit"]),
        ("cloud", &["drive", "google", "oauth", "token", "invite", "picker", "folder", "upload", "encrypt", "refresh"]),
        ("build", &["cargo", "linker", "gcc", "webkit", "tauri", "node", "nvm", "vite", "build", "release"]),
    ]),
    ("Website (test)", 0.22, &[
        ("pages", &["page", "route", "layout", "header", "footer", "hero", "section", "link", "nav", "slug"]),
        ("styles", &["css", "theme", "color", "font", "spacing", "dark", "contrast", "token", "grid", "responsive"]),
        ("deploy", &["deploy", "cdn", "cache", "domain", "dns", "preview", "build", "env", "secret", "rollback"]),
        ("seo", &["seo", "meta", "sitemap", "title", "description", "crawl", "index", "schema", "canonical", "og"]),
    ]),
    ("Mobile app (test)", 0.18, &[
        ("auth", &["login", "session", "token", "biometric", "keychain", "logout", "signup", "password", "otp", "refresh"]),
        ("offline", &["offline", "queue", "retry", "sqlite", "cache", "sync", "conflict", "network", "background", "battery"]),
        ("ui", &["screen", "navigation", "gesture", "animation", "list", "tab", "modal", "haptic", "safe-area", "keyboard"]),
    ]),
    ("Infra (test)", 0.12, &[
        ("k8s", &["cluster", "pod", "deployment", "ingress", "helm", "node", "autoscale", "namespace", "probe", "rollout"]),
        ("observability", &["metrics", "logs", "trace", "alert", "dashboard", "latency", "error", "budget", "slo", "oncall"]),
    ]),
    ("Dotfiles (test)", 0.08, &[
        ("shell", &["fish", "alias", "prompt", "path", "completion", "abbr", "function", "history", "env", "plugin"]),
        ("editor", &["neovim", "lsp", "keymap", "plugin", "theme", "treesitter", "format", "lint", "snippet", "telescope"]),
    ]),
    ("Me", 0.06, &[
        ("style", &["explain", "plain", "words", "short", "jargon", "summary", "example", "bullet", "tone", "detail"]),
        ("tools", &["fish", "bash", "terminal", "editor", "commit", "branch", "review", "test", "format", "lint"]),
    ]),
];

/// Marks loner test memories (not a word, so it doesn't make them related).
const LONER_MARK: &str = "–";
const SOURCES: &[&str] = &["claude-code", "claude-code", "claude-code", "codex", "codex", "cursor"];
const VERBS: &[&str] = &["needs", "breaks without", "depends on", "should use", "is faster with", "fails when missing", "replaces", "wraps", "checks", "updates"];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let fresh = args.iter().any(|a| a == "--fresh");
    let path = args.iter().find(|a| !a.starts_with("--")).expect("usage: seed_test_brain [--fresh] <path to astralis.db>");
    let mut s = Store::open(path).expect("couldn't open the database");
    let test_projects: Vec<_> = s.brain_projects().unwrap().into_iter().filter(|p| p.name.ends_with("(test)") && p.count > 0).collect();
    if !test_projects.is_empty() {
        if !fresh {
            eprintln!("This Brain already has test memories. Run again with --fresh to replace them.");
            std::process::exit(1);
        }
        // Remove the old test memories (only the "(test)" projects, and test entries in Me).
        let mut removed = 0;
        for p in &test_projects {
            for m in s.brain_memories(Some(p.folder)).unwrap() {
                s.delete_note(m.id).unwrap();
                removed += 1;
            }
        }
        let me = s.brain_me().unwrap();
        for m in s.brain_memories(Some(me)).unwrap().into_iter().filter(|m| m.hook.contains(", case ") || m.content.trim() == LONER_MARK || m.hook.starts_with("Loner ") || m.content.starts_with("Noted while working on")) {
            s.delete_note(m.id).unwrap();
            removed += 1;
        }
        println!("removed {removed} old test memories");
    }
    let mut rng = Rng(0xb4a1_7e57_0000_1001);
    let total = 1000usize;
    let mut made = 0usize;
    let mut handles: Vec<String> = Vec::with_capacity(total);

    for (pi, (project, share, topics)) in PROJECTS.iter().enumerate() {
        let folder = if *project == "Me" {
            s.brain_me().unwrap()
        } else {
            s.brain_project(&format!("test:{project}"), project).unwrap()
        };
        let target = if pi == PROJECTS.len() - 1 { total - made } else { (total as f64 * share).round() as usize };
        let mut here = 0usize;
        let mut attempts = 0usize;
        while here < target && attempts < target * 6 {
            attempts += 1;
            let kind = match rng.below(10) {
                0 | 1 => MemoryKind::Decision,
                2 | 3 => MemoryKind::Gotcha,
                4 | 5 => MemoryKind::Howto,
                6 | 7 => MemoryKind::Fact,
                8 => MemoryKind::Context,
                _ => MemoryKind::Preference,
            };
            let (hook, details) = if rng.chance(0.1) {
                // A loner: made-up words, related to nothing (an unconnected neuron).
                let word = |rng: &mut Rng| -> String {
                    const C: &[u8] = b"bdfgjklmnprstvzx";
                    const V: &[u8] = b"aeiou";
                    (0..3).map(|_| format!("{}{}", C[rng.below(C.len())] as char, V[rng.below(V.len())] as char)).collect()
                };
                // (The "–" marks it as test data for --fresh; it isn't a word, so it links nothing.)
                (format!("{} {} {}", word(&mut rng), word(&mut rng), word(&mut rng)), LONER_MARK.to_string())
            } else {
                let (topic, words) = topics[rng.below(topics.len())];
                let a = rng.pick(words);
                let mut b = rng.pick(words);
                while b == a {
                    b = rng.pick(words);
                }
                let c = rng.pick(words);
                let verb = rng.pick(VERBS);
                (
                    format!("The {a} {verb} the {b} in {topic}, case {}", here + 1),
                    format!("Seen in the {topic} work: {a}, {b} and {c} interact here.\nCheck the {c} first next time."),
                )
            };
            let source = SOURCES[rng.below(SOURCES.len())];
            let (m, merged) = s.remember(folder, kind, &hook, &details, source).unwrap();
            if merged {
                continue; // too close to one we made already; try again
            }
            here += 1;
            made += 1;
            if rng.chance(0.7) {
                s.set_memory_reviewed(&m.short, true).unwrap();
            }
            handles.push(m.short.clone());
            if made % 250 == 0 {
                println!("made {made} memories…");
            }
        }
        // A code map for each project.
        if *project != "Me" {
            let entries: Vec<MapEntry> = topics
                .iter()
                .flat_map(|(topic, words)| {
                    (0..4).map(move |i| MapEntry { path: format!("src/{topic}/{}.rs", words[i]), purpose: format!("{} handling for {topic}", words[i]), mtime: None })
                })
                .collect();
            s.brain_map_note(folder, &entries, "claude-code").unwrap();
        }
    }

    // Pin a few; recall some a lot (bigger, busier neurons), most a little, many never.
    for h in &handles {
        let recalls = match rng.below(20) {
            0 => 12 + rng.below(18),
            1..=4 => 3 + rng.below(6),
            5..=10 => 1,
            _ => 0,
        };
        for _ in 0..recalls {
            s.brain_read(h).unwrap();
        }
    }
    for h in handles.iter().step_by(97) {
        let m = s.brain_memory(h).unwrap();
        s.set_pinned(m.id, true).unwrap();
    }

    let g = s.brain_graph().unwrap();
    let mut degree = std::collections::HashMap::new();
    for l in &g.links {
        *degree.entry(l.a).or_insert(0) += 1;
        *degree.entry(l.b).or_insert(0) += 1;
    }
    let isolated = g.nodes.iter().filter(|n| !degree.contains_key(&n.id)).count();
    println!(
        "Made {} test memories ({} neurons in the graph, {} connections, {} with no connections).",
        made,
        g.nodes.len(),
        g.links.len(),
        isolated
    );
}
