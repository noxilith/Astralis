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

    // Before pairing: a note in A's own vault, and a "Team" vault with a note in it.
    let (private, team, n) = {
        let mut s = a.store.lock().unwrap();
        let private = s.create_note(NewNote { title: "Private".into(), body: "mine".into(), ..Default::default() }).unwrap();
        let team = s.create_vault("Team", None).unwrap();
        let n = s
            .create_note(NewNote { title: "Shared".into(), body: "hello".into(), vault_id: Some(team.id), ..Default::default() })
            .unwrap();
        (private, team, n)
    };

    // Wrong code is rejected; the right one pairs both ways.
    let code = a.net.start_pairing();
    let wrong = if code == "000000" { "111111" } else { "000000" };
    assert!(b.net.join(wrong, Some(a.net.addr())).await.is_err());
    let peer = b.net.join(&code, Some(a.net.addr())).await.unwrap();
    assert_eq!(peer.name, "Laptop");
    assert!(b.net.peers().iter().any(|p| p.id == a.net.id() && p.trusted));
    eventually("A trusts B", || a.net.peers().iter().any(|p| p.id == b.net.id() && p.trusted && p.connected)).await;

    // Paired, but nothing is shared yet: nothing moves.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(b.store.lock().unwrap().get_note(n.id).is_err());

    // A shares Team: it shows up on B as its own vault; A's private note never does.
    a.store.lock().unwrap().share_vault(team.id, &b.net.id()).unwrap();
    a.net.shares_changed(&b.net.id());
    eventually("note synced to B", || b.store.lock().unwrap().get_note(n.id).is_ok()).await;
    let b_vaults = b.store.lock().unwrap().list_vaults().unwrap();
    assert_eq!(b_vaults.len(), 2, "B's own vault plus Team");
    assert_eq!(b_vaults.iter().find(|v| v.id == team.id).unwrap().owner.as_deref(), Some(a.net.id().as_str()));
    assert!(b.store.lock().unwrap().get_note(private.id).is_err());
    assert_eq!(b.store.lock().unwrap().get_note(n.id).unwrap().body, "hello");

    // Later edits flow automatically, both ways.
    b.store
        .lock()
        .unwrap()
        .update_note(n.id, NotePatch { title: Some("Shared (edited on B)".into()), ..Default::default() })
        .unwrap();
    eventually("title synced to A", || a.store.lock().unwrap().get_note(n.id).unwrap().title == "Shared (edited on B)").await;

    // Live text: A's editor types and sends the update; B merges it immediately.
    let update = {
        let s = a.store.lock().unwrap();
        s.update_note(n.id, NotePatch { body: Some("hello world".into()), ..Default::default() }).unwrap();
        s.note_crdt(n.id).unwrap()
    };
    a.net.broadcast(Live::Update { note_id: n.id.to_string(), update });
    eventually("live text on B", || b.store.lock().unwrap().get_note(n.id).unwrap().body == "hello world").await;
    assert!(b.events.lock().unwrap().iter().any(|e| matches!(e, NetEvent::Live { live: Live::Update { .. }, .. })));

    // Presence: B's caret shows up on A with B's name.
    b.net.broadcast(Live::Presence { note_id: Some(n.id.to_string()), anchor: Some(3), head: Some(3), pointer: None });
    eventually("presence on A", || {
        a.events.lock().unwrap().iter().any(|e| {
            matches!(e, NetEvent::Live { name, live: Live::Presence { head: Some(3), .. }, .. } if name == "Desktop")
        })
    })
    .await;

    // B picks a color and a new name: A sees both right away, and remembers them.
    b.net.set_color("#FF0000").unwrap();
    assert!(b.net.set_color("red; background: url(x)").is_err(), "only hex colors");
    b.net.set_name("Noxi").unwrap();
    eventually("profile on A", || a.net.peers().iter().any(|p| p.id == b.net.id() && p.color == "#ff0000" && p.name == "Noxi")).await;
    let saved = a.store.lock().unwrap().get_setting("p2p.trusted").unwrap().unwrap();
    assert!(saved.contains("#ff0000"), "kept for when B is offline");

    // A stops sharing: B's copy disappears (A's stays). Sharing again brings it back.
    a.store.lock().unwrap().unshare_vault(team.id, &b.net.id()).unwrap();
    a.net.shares_changed(&b.net.id());
    eventually("B lost Team", || b.store.lock().unwrap().get_vault(team.id).is_err()).await;
    assert!(a.store.lock().unwrap().get_note(n.id).is_ok());
    a.store.lock().unwrap().share_vault(team.id, &b.net.id()).unwrap();
    a.net.shares_changed(&b.net.id());
    eventually("Team back on B", || b.store.lock().unwrap().get_note(n.id).is_ok()).await;

    // B leaves: gone on B, A stops sharing with B.
    b.store.lock().unwrap().leave_vault(team.id).unwrap();
    b.net.shares_changed(&a.net.id());
    eventually("A sees B left", || a.store.lock().unwrap().get_vault(team.id).unwrap().shared_with.is_empty()).await;
    assert!(b.store.lock().unwrap().get_vault(team.id).is_err());
    a.store.lock().unwrap().share_vault(team.id, &b.net.id()).unwrap();
    a.net.shares_changed(&b.net.id());
    eventually("Team shared again", || b.store.lock().unwrap().get_note(n.id).is_ok()).await;

    // A live edit of A's private note never reaches B.
    let update = a.store.lock().unwrap().note_crdt(private.id).unwrap();
    a.net.broadcast(Live::Update { note_id: private.id.to_string(), update });

    // Unpairing hangs up, and the other device forgets us too.
    b.net.unpair(&a.net.id()).unwrap();
    // B loses the vault A shared; A keeps it.
    assert!(b.store.lock().unwrap().get_vault(team.id).is_err());
    assert_eq!(b.store.lock().unwrap().list_vaults().unwrap().len(), 1);
    assert_eq!(a.store.lock().unwrap().get_note(n.id).unwrap().body, "hello world");
    assert!(!b.net.peers().iter().any(|p| p.id == a.net.id() && p.trusted));
    eventually("A forgets B", || !a.net.peers().iter().any(|p| p.id == b.net.id() && (p.trusted || p.connected))).await;
    eventually("B disconnected", || b.net.connected_count() == 0).await;

    // Nothing flows any more, either way.
    let from_b = b.store.lock().unwrap().create_note(NewNote { title: "after unpair".into(), ..Default::default() }).unwrap();
    a.store.lock().unwrap().update_note(n.id, NotePatch { title: Some("A only".into()), ..Default::default() }).unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert!(a.store.lock().unwrap().get_note(from_b.id).is_err());
    assert!(b.store.lock().unwrap().get_note(n.id).is_err());
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

/// Real mDNS discovery between two devices on this machine's network.
/// Ignored by default (needs multicast); run with `cargo test -p astralis-net -- --ignored`.
#[tokio::test]
#[ignore]
async fn devices_find_each_other_on_the_lan() {
    let mk = |name: &'static str| async move {
        let store: StoreHandle = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        store.lock().unwrap().set_setting(NAME_KEY, name).unwrap();
        Net::start(store, SecretKey::generate(), NetOptions { relays: false, mdns: true }, Arc::new(|_| {})).await.unwrap()
    };
    let a = mk("Laptop").await;
    let b = mk("Desktop").await;
    for _ in 0..200 {
        if b.peers().iter().any(|p| p.id == a.id() && p.nearby && p.name == "Laptop") {
            // And pairing works purely through discovery (no address given).
            let code = a.start_pairing();
            let peer = b.join(&code, None).await.unwrap();
            assert_eq!(peer.name, "Laptop");
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("devices didn't discover each other via mDNS");
}

#[test]
fn invites_round_trip() {
    let key = SecretKey::generate();
    let id = key.public();
    let invite = format!("astralis:{id}:123456");
    assert_eq!(parse_invite(&invite), Some((id, "123456".into())));
    assert_eq!(parse_invite(&format!(" astralis:{id}:\n123 456 ")), Some((id, "123456".into())));
    assert_eq!(parse_invite("123456"), None);
    assert_eq!(parse_invite("astralis:not-an-id:123456"), None);
}

/// Pairing and syncing between two devices that only know each other's identity — through
/// n0's public relays and DNS, like two devices on different networks.
/// Needs internet; run with `cargo test -p astralis-net -- --ignored`.
#[tokio::test]
#[ignore]
async fn pair_and_sync_across_the_internet() {
    let mk = |name: &'static str| async move {
        let store: StoreHandle = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        store.lock().unwrap().set_setting(NAME_KEY, name).unwrap();
        let net = Net::start(store.clone(), SecretKey::generate(), NetOptions { relays: true, mdns: false }, Arc::new(|_| {}))
            .await
            .unwrap();
        (net, store)
    };
    let (a, sa) = mk("Home").await;
    let (b, sb) = mk("Work").await;
    assert!(a.wait_online(Duration::from_secs(20)).await, "A couldn't reach the relay");
    assert!(b.wait_online(Duration::from_secs(20)).await, "B couldn't reach the relay");

    let (vault, n) = {
        let mut s = sa.lock().unwrap();
        let v = s.create_vault("Shared", None).unwrap();
        let n = s.create_note(NewNote { title: "Far away".into(), vault_id: Some(v.id), ..Default::default() }).unwrap();
        (v.id, n)
    };
    let code = a.start_pairing();
    // Only the invite text crosses over — no addresses.
    let peer = b.join(&a.invite(&code), None).await.unwrap();
    assert_eq!(peer.name, "Home");
    sa.lock().unwrap().share_vault(vault, &b.id()).unwrap();
    a.shares_changed(&b.id());
    for _ in 0..200 {
        if sb.lock().unwrap().get_note(n.id).is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("note didn't sync across the internet");
}

#[tokio::test]
async fn invited_by_email_joins_a_cloud_vault_without_pairing() {
    let a = device("Owner").await;
    let b = device("Friend").await;
    let (vault, n) = {
        let mut s = a.store.lock().unwrap();
        let v = s.create_vault("Trip", None).unwrap();
        s.set_cloud_vault(&astralis_core::cloud::CloudVault {
            vault_id: v.id,
            provider: "google-drive".into(),
            folder_id: "folder-1".into(),
            sync_key: vec![7; 32],
            owned: true,
            uploaded_vv: Default::default(),
        })
        .unwrap();
        s.add_cloud_invite(v.id, "friend@example.com", "s3cret").unwrap();
        let n = s.create_note(NewNote { title: "Packing".into(), body: "tent".into(), vault_id: Some(v.id), ..Default::default() }).unwrap();
        (v.id, n)
    };
    let private = a.store.lock().unwrap().create_note(NewNote { title: "Private".into(), ..Default::default() }).unwrap();

    // A wrong secret gets nowhere (and leaves nothing behind).
    let bad = JoinRequest { vault, email: "friend@example.com".into(), secret: "nope".into() };
    assert!(b.net.join_vault_at(&a.net.id(), bad, Some(a.net.addr())).await.is_err());
    assert!(b.store.lock().unwrap().vault_owner(vault).unwrap().is_none());

    // The right one: B gets the vault (and where it lives on the drive), not A's other vaults,
    // and isn't paired with A.
    let ok = JoinRequest { vault, email: "Friend@Example.com".into(), secret: "s3cret".into() };
    b.net.join_vault_at(&a.net.id(), ok.clone(), Some(a.net.addr())).await.unwrap();
    eventually("vault on B", || b.store.lock().unwrap().get_note(n.id).is_ok()).await;
    eventually("drive info on B", || b.store.lock().unwrap().cloud_vault(vault).unwrap().is_some_and(|c| c.folder_id == "folder-1" && !c.owned && c.sync_key == vec![7; 32])).await;
    assert!(b.store.lock().unwrap().get_note(private.id).is_err());
    assert!(!a.net.peers().iter().any(|p| p.id == b.net.id() && p.trusted), "joining isn't pairing");
    assert_eq!(a.store.lock().unwrap().cloud_invites(vault).unwrap()[0].accepted_by.as_deref(), Some(b.net.id().as_str()));

    // Edits flow both ways.
    b.store.lock().unwrap().update_note(n.id, NotePatch { title: Some("Packing list".into()), ..Default::default() }).unwrap();
    eventually("B's edit on A", || a.store.lock().unwrap().get_note(n.id).unwrap().title == "Packing list").await;

    // The invite can't be reused by someone else.
    let c = device("Stranger").await;
    assert!(c.net.join_vault_at(&a.net.id(), ok, Some(a.net.addr())).await.is_err());

    // A takes B out: B's copy goes; A's stays.
    let device = a.store.lock().unwrap().remove_cloud_invite(vault, "friend@example.com").unwrap();
    assert_eq!(device.as_deref(), Some(b.net.id().as_str()));
    a.net.shares_changed(&b.net.id());
    eventually("B lost the vault", || b.store.lock().unwrap().get_vault(vault).is_err()).await;
    assert!(b.store.lock().unwrap().cloud_vault(vault).unwrap().is_none());
    assert!(a.store.lock().unwrap().get_note(n.id).is_ok());
}
