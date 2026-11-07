use std::sync::{Arc, Mutex};
use std::time::Duration;

use astralis_core::Store;
use astralis_core::notes::{NewNote, NotePatch};

use super::*;

struct Device {
    net: Net,
    store: StoreHandle,
    events: Arc<Mutex<Vec<NetEvent>>>,
}

async fn device(name: &str) -> Device {
    let store: StoreHandle = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    store.lock().unwrap().set_setting(NAME_KEY, name).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let net = Net::start(
        store.clone(),
        SecretKey::generate(),
        NetOptions { relays: false, mdns: false },
        Arc::new(move |e| sink.lock().unwrap().push(e)),
    )
    .await
    .unwrap();
    Device { net, store, events }
}

/// Polls until `f` is true (or fails after a few seconds).
async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..100 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for: {what}");
}

#[tokio::test]
async fn pair_sync_and_collaborate_live() {
    let a = device("Laptop").await;
    let b = device("Desktop").await;

    // A note on A before pairing.
    let n = a
        .store
        .lock()
        .unwrap()
        .create_note(NewNote { title: "Shared".into(), body: "hello".into(), ..Default::default() })
        .unwrap();

    // Wrong code is rejected; the right one pairs both ways.
    let code = a.net.start_pairing();
    let wrong = if code == "000000" { "111111" } else { "000000" };
    assert!(b.net.join(wrong, Some(a.net.addr())).await.is_err());
    let peer = b.net.join(&code, Some(a.net.addr())).await.unwrap();
    assert_eq!(peer.name, "Laptop");
    assert!(b.net.peers().iter().any(|p| p.id == a.net.id() && p.trusted));
    eventually("A trusts B", || a.net.peers().iter().any(|p| p.id == b.net.id() && p.trusted && p.connected)).await;

    // Initial sync: B receives A's note.
    eventually("note synced to B", || b.store.lock().unwrap().get_note(n.id).is_ok()).await;
    assert_eq!(b.store.lock().unwrap().get_note(n.id).unwrap().body, "hello");

    // Later edits flow automatically, both ways.
    b.store
        .lock()
        .unwrap()
        .update_note(n.id, NotePatch { title: Some("Shared (edited on B)".into()), ..Default::default() })
        .unwrap();
    eventually("title synced to A", || a.store.lock().unwrap().get_note(n.id).unwrap().title == "Shared (edited on B)").await;

    // Live text: A types, sends its CRDT state; B merges it immediately.
    a.store.lock().unwrap().update_note(n.id, NotePatch { body: Some("hello world".into()), ..Default::default() }).unwrap();
    let state = a.store.lock().unwrap().note_text_state(n.id).unwrap().unwrap();
    a.net.broadcast(Live::Text { note_id: n.id.to_string(), state });
    eventually("live text on B", || b.store.lock().unwrap().get_note(n.id).unwrap().body == "hello world").await;
    assert!(b.events.lock().unwrap().iter().any(|e| matches!(e, NetEvent::RemoteText { body, .. } if body == "hello world")));

    // Presence: B's caret shows up on A with B's name.
    b.net.broadcast(Live::Presence { note_id: Some(n.id.to_string()), anchor: Some(3), head: Some(3), pointer: None });
    eventually("presence on A", || {
        a.events.lock().unwrap().iter().any(|e| {
            matches!(e, NetEvent::Live { name, live: Live::Presence { head: Some(3), .. }, .. } if name == "Desktop")
        })
    })
    .await;

    // Unpairing disconnects.
    b.net.unpair(&a.net.id()).unwrap();
    assert!(!b.net.peers().iter().any(|p| p.id == a.net.id() && p.trusted));
}

#[tokio::test]
async fn pairing_code_is_single_use_and_rate_limited() {
    let a = device("A").await;
    let b = device("B").await;
    let c = device("C").await;
    let code = a.net.start_pairing();
    b.net.join(&code, Some(a.net.addr())).await.unwrap();
    // Same code can't pair a second device.
    assert!(c.net.join(&code, Some(a.net.addr())).await.is_err());

    // Too many wrong guesses cancel pairing.
    let code = a.net.start_pairing();
    for i in 0..MAX_PAIR_ATTEMPTS {
        let guess = format!("{:06}", (code.parse::<u32>().unwrap() + 1 + i) % 1_000_000);
        assert!(c.net.join(&guess, Some(a.net.addr())).await.is_err());
    }
    assert!(c.net.join(&code, Some(a.net.addr())).await.is_err(), "pairing was cancelled");
}
