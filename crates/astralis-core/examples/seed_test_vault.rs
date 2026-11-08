//! Creates a "Test galaxy" vault with 1,000 notes for trying the graph at scale:
//! - 600 in one big linked web (hubs, chains and cross-links),
//! - 200 in 25 small clusters that only link among themselves,
//! - 200 with no links at all.
//!
//! Usage: `cargo run -p astralis-core --example seed_test_vault -- <path to astralis.db>`
//! (or `./run.sh seed-test-vault`). Deterministic: the same notes every time.

use astralis_core::Store;
use astralis_core::notes::{NewNote, NotePatch};

const ADJ: &[&str] = &[
    "Quiet", "Silver", "Distant", "Burning", "Hidden", "Frozen", "Golden", "Wandering", "Crimson", "Electric",
    "Ancient", "Hollow", "Radiant", "Drifting", "Violet", "Restless", "Lunar", "Solar", "Velvet", "Infinite",
];
const NOUN: &[&str] = &[
    "Nebula", "Comet", "Orbit", "Quasar", "Pulsar", "Horizon", "Eclipse", "Meteor", "Aurora", "Zenith",
    "Nova", "Cosmos", "Halo", "Corona", "Vortex", "Signal", "Archive", "Garden", "Lantern", "Harbor",
];
const TAGS: &[&str] = &["research", "ideas", "project", "journal", "reference", "todo"];

/// Small deterministic random numbers.
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
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: seed_test_vault <path to astralis.db>");
    let mut store = Store::open(&path).expect("couldn't open the database");
    if store.list_vaults().unwrap().iter().any(|v| v.name == "Test galaxy") {
        eprintln!("There's already a \"Test galaxy\" vault here; delete it in the app first to make a fresh one.");
        std::process::exit(1);
    }
    let vault = store.create_vault("Test galaxy", None).expect("couldn't create the vault");
    let mut rng = Rng(0x5eed_a57e_a115_u64);

    // 1,000 distinct titles.
    let titles: Vec<String> = (0..1000)
        .map(|i| {
            let a = ADJ[i % ADJ.len()];
            let n = NOUN[(i / ADJ.len()) % NOUN.len()];
            let round = i / (ADJ.len() * NOUN.len());
            if round == 0 { format!("{a} {n}") } else { format!("{a} {n} {}", round + 1) }
        })
        .collect();

    // Links, by index. Web: 0..600 (hubs 0..12), clusters: 600..800, unconnected: 800..1000.
    let mut links: Vec<Vec<usize>> = vec![Vec::new(); 1000];
    const HUBS: usize = 12;
    for h in 0..HUBS {
        links[h].push((h + 1) % HUBS); // hubs in a ring
    }
    for i in HUBS..600 {
        let hub = i % HUBS;
        if rng.chance(0.7) {
            links[i].push(hub);
        }
        if rng.chance(0.55) {
            links[i].push(i - 1); // chains along the way
        }
        if rng.chance(0.18) {
            links[i].push(rng.below(600)); // the odd long-range link
        }
        if links[i].is_empty() {
            links[i].push(hub);
        }
    }
    for c in 0..25 {
        let start = 600 + c * 8;
        for j in 0..8 {
            let i = start + j;
            links[i].push(start + (j + 1) % 8);
            if rng.chance(0.4) {
                links[i].push(start + rng.below(8));
            }
        }
    }

    // Create every note first (empty), then fill in bodies so every [[link]] resolves.
    let mut ids = Vec::with_capacity(1000);
    for (i, title) in titles.iter().enumerate() {
        let tags = if i < 600 && rng.chance(0.35) {
            vec![TAGS[i % HUBS % TAGS.len()].to_string()]
        } else if (600..800).contains(&i) && i % 8 == 0 {
            vec![format!("cluster-{}", (i - 600) / 8 + 1)]
        } else {
            vec![]
        };
        let note = store
            .create_note(NewNote {
                title: title.clone(),
                body: String::new(),
                tags,
                pinned: i < 3,
                vault_id: Some(vault.id),
                ..Default::default()
            })
            .expect("couldn't create a note");
        ids.push(note.id);
        if (i + 1) % 250 == 0 {
            println!("created {} notes…", i + 1);
        }
    }
    for i in 0..1000 {
        let mut body = format!("# {}\n\nA test note for the graph.", titles[i]);
        let mut targets = links[i].clone();
        targets.sort_unstable();
        targets.dedup();
        targets.retain(|&t| t != i);
        if !targets.is_empty() {
            body.push_str("\n\nRelated: ");
            body.push_str(&targets.iter().map(|&t| format!("[[{}]]", titles[t])).collect::<Vec<_>>().join(", "));
        }
        store
            .update_note(ids[i], NotePatch { body: Some(body), ..Default::default() })
            .expect("couldn't write a note");
    }

    let graph = store.graph(Some(vault.id), true).unwrap();
    let linked = graph.nodes.iter().filter(|n| n.degree > 0).count();
    println!(
        "Made \"Test galaxy\": {} notes, {} links, {} linked, {} with no links.",
        graph.nodes.len(),
        graph.edges.len(),
        linked,
        graph.nodes.len() - linked
    );
}
