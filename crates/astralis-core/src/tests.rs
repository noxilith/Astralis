use std::{thread::sleep, time::Duration};

use uuid::Uuid;

use crate::ai::Brain;
use crate::calendar::{CalendarKind, EventInput};
use crate::grammar::Grammar;
use crate::links::parse_wikilinks;
use crate::merge::MergeRequest;
use crate::notes::{ArchiveFilter, ListFilter, NewNote, Note, NoteKind, NotePatch};
use crate::planners::PlannerInput;
use crate::search::{HIGHLIGHT_END, HIGHLIGHT_START};
use crate::tasks::{
    Frequency, NewTask, Recurrence, ReminderKind, TaskFilter, TaskPatch, TaskStatus,
};
use crate::templates::TemplateInput;
use crate::sync::{ShareRole, SqlValue, Tombstone};
use crate::*;

const DAY: i64 = 86_400_000;

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

fn tick() {
    sleep(Duration::from_millis(3));
}

fn note(s: &Store, title: &str, body: &str) -> Note {
    s.create_note(NewNote {
        title: title.into(),
        body: body.into(),
        ..Default::default()
    })
    .unwrap()
}

fn note_in(s: &Store, vault: Uuid, title: &str, body: &str) -> Note {
    s.create_note(NewNote {
        title: title.into(),
        body: body.into(),
        vault_id: Some(vault),
        ..Default::default()
    })
    .unwrap()
}

fn ids(v: Vec<Note>) -> Vec<Uuid> {
    v.into_iter().map(|n| n.id).collect()
}

fn search(s: &Store, q: &str, archived: bool) -> Vec<crate::search::SearchHit> {
    s.search(None, q, archived, 10).unwrap()
}

// ---------- notes ----------

#[test]
fn quick_note_with_no_input() {
    let s = store();
    let n = s.create_note(NewNote::default()).unwrap();
    assert_eq!(n.title, "");
    assert_eq!(n.kind, NoteKind::Note);
    assert_eq!(n.vault_id, s.default_vault().unwrap());
    assert!(!n.pinned && !n.is_archived() && !n.encrypted);
    assert_eq!(s.get_note(n.id).unwrap(), n);
}

#[test]
fn quick_capture_splits_title() {
    let s = store();
    let n = s.quick_capture(None, "# Groceries\nmilk\neggs").unwrap();
    assert_eq!(
        (n.title.as_str(), n.body.as_str()),
        ("Groceries", "milk\neggs")
    );
}

#[test]
fn update_changes_fields_and_timestamp() {
    let s = store();
    let n = note(&s, "a", "b");
    tick();
    let u = s
        .update_note(
            n.id,
            NotePatch {
                body: Some("c".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!((u.title.as_str(), u.body.as_str()), ("a", "c"));
    assert!(u.updated_at > n.updated_at);
    assert_eq!(s.update_note(n.id, NotePatch::default()).unwrap(), u);
}

#[test]
fn pin_and_list_order() {
    let s = store();
    let a = note(&s, "a", "");
    tick();
    let b = note(&s, "b", "");
    assert_eq!(
        ids(s.list_notes(&ListFilter::default()).unwrap()),
        vec![b.id, a.id]
    );
    s.set_pinned(a.id, true).unwrap();
    assert_eq!(
        ids(s.list_notes(&ListFilter::default()).unwrap()),
        vec![a.id, b.id]
    );
    let pinned = s
        .list_notes(&ListFilter {
            pinned_only: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(ids(pinned), vec![a.id]);
}

#[test]
fn archive_roundtrip_and_filters() {
    let s = store();
    let n = note(&s, "x", "");
    let archived = s.archive_note(n.id).unwrap();
    assert!(archived.is_archived());
    tick();
    assert_eq!(
        s.archive_note(n.id).unwrap().archived_at,
        archived.archived_at
    );
    assert!(s.list_notes(&ListFilter::default()).unwrap().is_empty());
    let only = ListFilter {
        archive: ArchiveFilter::Archived,
        ..Default::default()
    };
    assert_eq!(s.list_notes(&only).unwrap().len(), 1);
    assert!(!s.unarchive_note(n.id).unwrap().is_archived());
}

#[test]
fn delete_and_not_found() {
    let s = store();
    let n = note(&s, "x", "");
    s.delete_note(n.id).unwrap();
    assert!(matches!(s.get_note(n.id), Err(Error::NotFound(..))));
    assert!(matches!(s.delete_note(n.id), Err(Error::NotFound(..))));
    assert!(matches!(
        s.set_pinned(Uuid::now_v7(), true),
        Err(Error::NotFound(..))
    ));
}

#[test]
fn tags_are_normalized_and_filterable() {
    let s = store();
    let n = s
        .create_note(NewNote {
            title: "t".into(),
            tags: vec!["#Work".into(), "big idea".into(), "work".into()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(n.tags, vec!["big-idea", "work"]);
    note(&s, "other", "");
    let work = s
        .list_notes(&ListFilter {
            tag: Some("WORK".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(ids(work), vec![n.id]);
    let n = s.remove_tag(n.id, "work").unwrap();
    assert_eq!(n.tags, vec!["big-idea"]);
    let tags = s.list_tags(None).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!((tags[0].name.as_str(), tags[0].count), ("big-idea", 1));
}

#[test]
fn persists_on_disk() {
    let dir = std::env::temp_dir().join(format!("astralis-test-{}", Uuid::now_v7()));
    let path = dir.join("sub/notes.db");
    let id = Store::open(&path)
        .unwrap()
        .create_note(NewNote {
            title: "kept".into(),
            ..Default::default()
        })
        .unwrap()
        .id;
    let s = Store::open(&path).unwrap();
    assert_eq!(s.get_note(id).unwrap().title, "kept");
    assert_eq!(s.list_templates().unwrap().len(), 4, "builtins seeded once");
    assert_eq!(
        s.list_vaults().unwrap().len(),
        1,
        "default vault created once"
    );
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A database from before vaults were workspaces upgrades cleanly.
#[test]
fn migrates_v2_database() {
    let dir = std::env::temp_dir().join(format!("astralis-test-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("old.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        for (i, sql) in crate::store::MIGRATIONS.iter().take(2).enumerate() {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", (i + 1) as i64)
                .unwrap();
        }
        conn.execute(
            "INSERT INTO notes(id, title, body, created_at, updated_at) VALUES (?1, 'Old', 'old body', 1, 1)",
            [Uuid::now_v7().as_bytes()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO whiteboards(id, title, scene, created_at, updated_at) VALUES (?1, 'Board', '{\"elements\":[]}', 1, 1)",
            [Uuid::now_v7().as_bytes()],
        )
        .unwrap();
    }
    let s = Store::open(&path).unwrap();
    let vaults = s.list_vaults().unwrap();
    assert_eq!(vaults.len(), 1);
    assert!(!vaults[0].encrypted);
    let notes = s.list_notes(&ListFilter::default()).unwrap();
    assert_eq!(notes.len(), 2);
    let board = notes
        .iter()
        .find(|n| n.kind == NoteKind::Whiteboard)
        .unwrap();
    assert_eq!(board.title, "Board");
    assert_eq!(search(&s, "old", false).len(), 1);
    assert!(
        search(&s, "elements", false).is_empty(),
        "scene JSON isn't indexed"
    );
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------- vaults as workspaces ----------

#[test]
fn vaults_have_separate_notes() {
    let mut s = store();
    let personal = s.default_vault().unwrap();
    let work = s.create_vault("Work", None).unwrap();
    assert!(!work.encrypted && work.unlocked);
    let a = note_in(&s, personal, "Plan", "see [[Shared]]");
    let w = note_in(&s, work.id, "Plan", "see [[Shared]]");
    let shared_work = note_in(&s, work.id, "Shared", "");
    s.set_tags(w.id, &["job".into()]).unwrap();

    let in_work = s
        .list_notes(&ListFilter {
            vault_id: Some(work.id),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(in_work.len(), 2);
    assert_eq!(
        ids(s.list_notes(&ListFilter::default()).unwrap()),
        vec![a.id]
    );

    // Links, backlinks, search, tags and the graph are per vault.
    assert!(s.outgoing_links(a.id).unwrap()[0].notes.is_empty());
    assert_eq!(
        s.outgoing_links(w.id).unwrap()[0].notes[0].id,
        shared_work.id
    );
    assert_eq!(s.backlinks(shared_work.id).unwrap().len(), 1);
    assert_eq!(s.search(Some(work.id), "plan", false, 10).unwrap().len(), 1);
    assert!(s.list_tags(None).unwrap().is_empty());
    assert_eq!(s.list_tags(Some(work.id)).unwrap().len(), 1);
    assert_eq!(s.graph(Some(work.id), false).unwrap().edges.len(), 1);
    assert!(s.graph(None, false).unwrap().edges.is_empty());

    // Moving between plain vaults keeps content and re-resolves links.
    let moved = s.move_note_to_vault(a.id, work.id).unwrap();
    assert_eq!(moved.vault_id, work.id);
    assert_eq!(s.backlinks(shared_work.id).unwrap().len(), 2);

    let names: Vec<String> = s
        .list_vaults()
        .unwrap()
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert_eq!(names, vec!["Personal", "Work"]);
    s.rename_vault(work.id, "Job").unwrap();
    s.delete_vault(work.id, None).unwrap();
    assert!(matches!(s.get_note(w.id), Err(Error::NotFound(..))));
    assert!(
        s.delete_vault(personal, None).is_err(),
        "can't delete the only vault"
    );
}

// ---------- whiteboards ----------

#[test]
fn whiteboards_are_notes() {
    let mut s = store();
    let w = s.create_whiteboard(None, "").unwrap();
    assert_eq!(
        (w.kind, w.title.as_str(), w.body.as_str()),
        (NoteKind::Whiteboard, "Untitled whiteboard", "{}")
    );
    s.save_whiteboard_scene(w.id, r#"{"elements":[{"text":"brainstorm"}]}"#)
        .unwrap();
    assert!(s.save_whiteboard_scene(w.id, "not json").is_err());
    assert!(
        s.note_history(w.id).unwrap().is_empty(),
        "no history for scenes"
    );
    assert!(
        search(&s, "brainstorm", false).is_empty(),
        "scene JSON isn't indexed"
    );

    // Titles are searchable and linkable; filters by kind work.
    s.update_note(
        w.id,
        NotePatch {
            title: Some("Roadmap".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(search(&s, "roadmap", false).len(), 1);
    let n = note(&s, "Text", "see [[Roadmap]]");
    assert_eq!(s.outgoing_links(n.id).unwrap()[0].notes[0].id, w.id);
    let boards = s
        .list_notes(&ListFilter {
            kind: Some(NoteKind::Whiteboard),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(ids(boards), vec![w.id]);

    // Whiteboards can live in encrypted vaults.
    let v = s.create_vault("Secret", Some("password1")).unwrap();
    let sw = s.create_whiteboard(Some(v.id), "Private board").unwrap();
    s.save_whiteboard_scene(sw.id, r#"{"elements":[1]}"#)
        .unwrap();
    assert_eq!(s.get_note(sw.id).unwrap().body, r#"{"elements":[1]}"#);
    s.lock_vault(v.id);
    assert!(matches!(s.get_note(sw.id), Err(Error::VaultLocked(_))));
    assert!(
        s.merge_notes(MergeRequest {
            note_ids: vec![w.id, n.id],
            title: None,
            archive_originals: true
        })
        .is_err()
    );
}

// ---------- links ----------

#[test]
fn parses_wikilinks() {
    let links =
        parse_wikilinks("see [[Alpha]], [[ Beta | b ]] and [[Gamma#Intro]]; not [[]] or [[a\nb]]");
    let targets: Vec<&str> = links.iter().map(|l| l.target.as_str()).collect();
    assert_eq!(targets, vec!["Alpha", "Beta", "Gamma"]);
}

#[test]
fn backlinks_outgoing_and_graph() {
    let s = store();
    let a = note(&s, "Alpha", "links to [[Beta]] and [[Missing]]");
    let b = note(&s, "Beta", "nothing");
    let c = note(&s, "Gamma", "also [[beta]]");
    note(&s, "Orphan", "");

    let back = s.backlinks(b.id).unwrap();
    let mut back_ids: Vec<Uuid> = back.iter().map(|l| l.note.id).collect();
    back_ids.sort();
    let mut expected = vec![a.id, c.id];
    expected.sort();
    assert_eq!(back_ids, expected);
    assert!(back.iter().any(|l| l.context.contains("[[Beta]]")));

    let out = s.outgoing_links(a.id).unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].notes[0].id, b.id);
    assert!(out[1].notes.is_empty(), "unresolved link");

    let g = s.graph(None, false).unwrap();
    assert_eq!(g.nodes.len(), 3);
    assert_eq!(g.edges.len(), 2);
    assert_eq!(s.graph(None, true).unwrap().nodes.len(), 4);
}

#[test]
fn renaming_rewrites_links() {
    let s = store();
    let a = note(&s, "Alpha", "see [[Beta|the b]] and [[beta#sec]]");
    let b = note(&s, "Beta", "");
    s.update_note(
        b.id,
        NotePatch {
            title: Some("Bravo".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        s.get_note(a.id).unwrap().body,
        "see [[Bravo|the b]] and [[Bravo#sec]]"
    );
    assert_eq!(s.backlinks(b.id).unwrap().len(), 1);
}

#[test]
fn title_suggestions() {
    let s = store();
    note(&s, "Project Apollo", "");
    note(&s, "Apple pie", "");
    let r = s.suggest_titles(None, "ap", 10).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].title, "Apple pie", "prefix match first");
}

// ---------- history ----------

#[test]
fn history_snapshots_and_restore() {
    let s = store();
    let n = note(&s, "v1", "first");
    s.update_note(
        n.id,
        NotePatch {
            body: Some("second".into()),
            ..Default::default()
        },
    )
    .unwrap();
    s.update_note(
        n.id,
        NotePatch {
            body: Some("third".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let h = s.note_history(n.id).unwrap();
    assert_eq!(h.len(), 1, "rapid edits coalesce into one version");
    assert_eq!(h[0].body, "first");

    let restored = s.restore_version(h[0].id).unwrap();
    assert_eq!(restored.body, "first");
    let h = s.note_history(n.id).unwrap();
    assert_eq!(h.len(), 2);
    assert_eq!(h[0].body, "third", "restore saved the pre-restore content");

    s.snapshot_note(n.id).unwrap();
    assert_eq!(s.note_history(n.id).unwrap().len(), 3);
}

// ---------- merge ----------

#[test]
fn merging_combines_and_archives() {
    let s = store();
    let a = s
        .create_note(NewNote {
            title: "A".into(),
            body: "aaa".into(),
            tags: vec!["x".into()],
            ..Default::default()
        })
        .unwrap();
    let b = s
        .create_note(NewNote {
            title: "B".into(),
            body: "bbb".into(),
            tags: vec!["y".into()],
            pinned: true,
            ..Default::default()
        })
        .unwrap();
    let linker = note(&s, "L", "to [[B]]");
    let m = s
        .merge_notes(MergeRequest {
            note_ids: vec![a.id, b.id],
            title: Some("AB".into()),
            archive_originals: true,
        })
        .unwrap();
    assert_eq!(m.body, "## A\n\naaa\n\n## B\n\nbbb");
    assert_eq!(m.tags, vec!["x", "y"]);
    assert!(m.pinned);
    assert!(s.get_note(a.id).unwrap().is_archived());
    assert_eq!(s.get_note(linker.id).unwrap().body, "to [[AB]]");
    assert!(
        s.merge_notes(MergeRequest {
            note_ids: vec![a.id],
            title: None,
            archive_originals: true
        })
        .is_err()
    );
}

// ---------- templates ----------

#[test]
fn templates_render() {
    let s = store();
    let journal = s
        .list_templates()
        .unwrap()
        .into_iter()
        .find(|t| t.name == "Daily Journal")
        .unwrap();
    let n = s.note_from_template(journal.id, None, None).unwrap();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    assert_eq!(n.title, format!("Journal — {today}"));
    assert!(!n.body.contains("{{"));

    let t = s
        .create_template(TemplateInput {
            name: "Mine".into(),
            title: "{{title}}!".into(),
            body: "hi {{title}}".into(),
        })
        .unwrap();
    let n = s
        .note_from_template(t.id, Some("Bob".into()), None)
        .unwrap();
    assert_eq!((n.title.as_str(), n.body.as_str()), ("Bob!", "hi Bob"));
    let from = s.template_from_note(n.id, "Copy").unwrap();
    assert_eq!(from.body, "hi Bob");
    s.delete_template(t.id).unwrap();
}

// ---------- tasks ----------

#[test]
fn tasks_order_and_complete() {
    let s = store();
    let low = s
        .create_task(NewTask {
            title: "low".into(),
            priority: 1,
            ..Default::default()
        })
        .unwrap();
    let high = s
        .create_task(NewTask {
            title: "high".into(),
            priority: 3,
            ..Default::default()
        })
        .unwrap();
    let list: Vec<Uuid> = s
        .list_tasks(&TaskFilter::default())
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(list, vec![high.id, low.id]);

    let done = s.complete_task(high.id).unwrap();
    assert_eq!(done.status, TaskStatus::Done);
    assert!(done.completed_at.is_some());
    assert_eq!(s.list_tasks(&TaskFilter::default()).unwrap().len(), 1);
    assert_eq!(
        s.list_tasks(&TaskFilter {
            include_done: true,
            ..Default::default()
        })
        .unwrap()
        .len(),
        2
    );
    assert_eq!(s.reopen_task(high.id).unwrap().status, TaskStatus::Todo);

    assert!(
        s.create_task(NewTask {
            title: " ".into(),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        s.create_task(NewTask {
            title: "x".into(),
            priority: 9,
            ..Default::default()
        })
        .is_err()
    );
}

#[test]
fn recurring_task_rolls_forward() {
    let s = store();
    let now = now_ms();
    let due = now + DAY / 2;
    let t = s
        .create_task(NewTask {
            title: "water plants".into(),
            due_at: Some(due),
            remind_at: Some(due - 3_600_000),
            recurrence: Some(Recurrence {
                freq: Frequency::Daily,
                interval: 2,
                until: None,
            }),
            ..Default::default()
        })
        .unwrap();
    let t2 = s.complete_task(t.id).unwrap();
    assert_eq!(t2.status, TaskStatus::Todo);
    assert_eq!(t2.completion_count, 1);
    let delta = t2.due_at.unwrap() - due;
    assert!(
        (2 * DAY - 3_600_000..=2 * DAY + 3_600_000).contains(&delta),
        "≈2 days (DST-tolerant): {delta}"
    );
    assert_eq!(
        t2.remind_at.unwrap() - t2.due_at.unwrap(),
        -3_600_000,
        "reminder shifted with it"
    );

    s.update_task(
        t.id,
        TaskPatch {
            recurrence: Some(Some(Recurrence {
                freq: Frequency::Daily,
                interval: 1,
                until: Some(now),
            })),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(s.complete_task(t.id).unwrap().status, TaskStatus::Done);
}

#[test]
fn overdue_recurring_task_skips_to_future() {
    let s = store();
    let now = now_ms();
    let t = s
        .create_task(NewTask {
            title: "weekly".into(),
            due_at: Some(now - 20 * DAY),
            recurrence: Some(Recurrence {
                freq: Frequency::Weekly,
                interval: 1,
                until: None,
            }),
            ..Default::default()
        })
        .unwrap();
    let next = s.complete_task(t.id).unwrap().due_at.unwrap();
    assert!(next > now && next <= now + 8 * DAY);
}

#[test]
fn monthly_recurrence_clamps_day() {
    use chrono::{Local, TimeZone};
    let jan31 = Local
        .with_ymd_and_hms(2027, 1, 31, 9, 0, 0)
        .unwrap()
        .timestamp_millis();
    let r = Recurrence {
        freq: Frequency::Monthly,
        interval: 1,
        until: None,
    };
    let feb = Local.timestamp_millis_opt(r.step(jan31)).unwrap();
    assert_eq!(feb.format("%m-%d %H:%M").to_string(), "02-28 09:00");
}

#[test]
fn task_patch_json_can_clear_fields() {
    let s = store();
    let t = s
        .create_task(NewTask {
            title: "x".into(),
            due_at: Some(5),
            ..Default::default()
        })
        .unwrap();
    let keep: TaskPatch = serde_json::from_str(r#"{"title":"y"}"#).unwrap();
    assert_eq!(s.update_task(t.id, keep).unwrap().due_at, Some(5));
    let clear: TaskPatch = serde_json::from_str(r#"{"dueAt":null}"#).unwrap();
    assert_eq!(s.update_task(t.id, clear).unwrap().due_at, None);
}

#[test]
fn reminders_fire_once_and_snooze() {
    let s = store();
    let now = now_ms();
    let t = s
        .create_task(NewTask {
            title: "call".into(),
            remind_at: Some(now - 1000),
            ..Default::default()
        })
        .unwrap();
    s.create_task(NewTask {
        title: "later".into(),
        remind_at: Some(now + DAY),
        ..Default::default()
    })
    .unwrap();
    let e = s
        .create_event(EventInput {
            title: "standup".into(),
            start_at: now,
            end_at: now + 1000,
            remind_at: Some(now - 10),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(s.due_reminders(now).unwrap().len(), 2);
    s.mark_reminded(ReminderKind::Task, t.id).unwrap();
    s.mark_reminded(ReminderKind::Event, e.id).unwrap();
    assert!(s.due_reminders(now).unwrap().is_empty());
    s.snooze_reminder(ReminderKind::Task, t.id, now + 500)
        .unwrap();
    assert!(s.due_reminders(now).unwrap().is_empty());
    assert_eq!(s.due_reminders(now + 600).unwrap().len(), 1);
}

// ---------- calendar & planners ----------

#[test]
fn calendar_combines_events_tasks_and_projections() {
    let s = store();
    let start = now_ms();
    let end = start + 7 * DAY;
    s.create_event(EventInput {
        title: "trip".into(),
        start_at: start - DAY,
        end_at: start + DAY,
        all_day: true,
        ..Default::default()
    })
    .unwrap();
    s.create_event(EventInput {
        title: "past".into(),
        start_at: start - 3 * DAY,
        end_at: start - 2 * DAY,
        ..Default::default()
    })
    .unwrap();
    s.create_task(NewTask {
        title: "daily".into(),
        due_at: Some(start + 1000),
        recurrence: Some(Recurrence {
            freq: Frequency::Daily,
            interval: 1,
            until: None,
        }),
        ..Default::default()
    })
    .unwrap();
    let items = s.calendar(start, end).unwrap();
    assert_eq!(
        items
            .iter()
            .filter(|i| i.kind == CalendarKind::Event)
            .count(),
        1
    );
    let tasks: Vec<_> = items
        .iter()
        .filter(|i| i.kind == CalendarKind::Task)
        .collect();
    assert_eq!(tasks.len(), 7);
    assert_eq!(tasks.iter().filter(|t| t.projected).count(), 6);
    assert!(
        s.create_event(EventInput {
            title: "bad".into(),
            start_at: 10,
            end_at: 5,
            ..Default::default()
        })
        .is_err()
    );
}

#[test]
fn planners_count_and_release_tasks() {
    let s = store();
    let p = s
        .create_planner(PlannerInput {
            name: "Launch".into(),
            ..Default::default()
        })
        .unwrap();
    let t = s
        .create_task(NewTask {
            title: "a".into(),
            planner_id: Some(p.id),
            ..Default::default()
        })
        .unwrap();
    s.create_task(NewTask {
        title: "b".into(),
        planner_id: Some(p.id),
        status: TaskStatus::Done,
        ..Default::default()
    })
    .unwrap();
    let p = s.get_planner(p.id).unwrap();
    assert_eq!((p.task_count, p.done_count), (2, 1));
    let doing = s
        .update_task(
            t.id,
            TaskPatch {
                status: Some(TaskStatus::Doing),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(doing.status, TaskStatus::Doing);
    s.delete_planner(p.id).unwrap();
    assert_eq!(s.get_task(t.id).unwrap().planner_id, None);
}

// ---------- encrypted vaults ----------

#[test]
fn encrypted_vault_encrypts_at_rest_and_locks() {
    let mut s = store();
    let v = s.create_vault("Private", Some("correct horse")).unwrap();
    assert!(v.encrypted && v.unlocked);
    let n = note_in(&s, v.id, "bank", "pin is 1234");
    assert!(n.encrypted);
    assert_eq!(n.body, "pin is 1234");

    let (title, body, cipher): (String, String, Vec<u8>) = s
        .conn
        .query_row(
            "SELECT title, body, cipher FROM notes WHERE id = ?1",
            [n.id.as_bytes()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(title.is_empty() && body.is_empty());
    assert!(!cipher.windows(4).any(|w| w == b"1234"));
    assert!(s.search(Some(v.id), "1234", true, 10).unwrap().is_empty());
    assert!(
        s.list_notes(&ListFilter::default()).unwrap().is_empty(),
        "not in the default vault"
    );

    s.update_note(
        n.id,
        NotePatch {
            body: Some("pin is 9999".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        s.note_history(n.id).unwrap().is_empty(),
        "no plaintext history"
    );
    assert!(s.set_tags(n.id, &["x".into()]).is_err());

    s.lock_vault(v.id);
    assert!(!s.get_vault(v.id).unwrap().unlocked);
    assert!(matches!(s.get_note(n.id), Err(Error::VaultLocked(_))));
    assert!(matches!(
        s.list_notes(&ListFilter {
            vault_id: Some(v.id),
            ..Default::default()
        }),
        Err(Error::VaultLocked(_))
    ));
    assert!(matches!(
        s.unlock_vault(v.id, "wrong password"),
        Err(Error::WrongPassword)
    ));
    s.unlock_vault(v.id, "correct horse").unwrap();
    assert_eq!(s.get_note(n.id).unwrap().body, "pin is 9999");

    s.change_vault_password(v.id, "correct horse", "battery staple")
        .unwrap();
    s.lock_all_vaults();
    assert!(s.unlock_vault(v.id, "correct horse").is_err());
    s.unlock_vault(v.id, "battery staple").unwrap();
    let in_vault = s
        .list_notes(&ListFilter {
            vault_id: Some(v.id),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(ids(in_vault), vec![n.id]);
    assert!(s.create_vault("weak", Some("short")).is_err());
}

#[test]
fn moving_into_encrypted_vault_scrubs_plaintext() {
    let mut s = store();
    let personal = s.default_vault().unwrap();
    let v = s.create_vault("V", Some("password1")).unwrap();
    let n = s
        .create_note(NewNote {
            title: "Diary".into(),
            body: "secret [[Other]]".into(),
            tags: vec!["me".into()],
            ..Default::default()
        })
        .unwrap();
    s.update_note(
        n.id,
        NotePatch {
            body: Some("secret v2 [[Other]]".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let mut brain = Brain::new(None);
    brain.index_all(&s).unwrap();

    let moved = s.move_note_to_vault(n.id, v.id).unwrap();
    assert!(moved.encrypted);
    assert_eq!(moved.body, "secret v2 [[Other]]");
    assert!(moved.tags.is_empty());
    let count = |sql: &str| -> i64 {
        s.conn
            .query_row(sql, [n.id.as_bytes()], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(
        count("SELECT COUNT(*) FROM note_versions WHERE note_id = ?1"),
        0
    );
    assert_eq!(count("SELECT COUNT(*) FROM links WHERE source_id = ?1"), 0);
    assert_eq!(
        count("SELECT COUNT(*) FROM embeddings WHERE note_id = ?1"),
        0
    );
    assert!(search(&s, "secret", true).is_empty());
    assert!(s.list_tags(None).unwrap().is_empty());
    assert_eq!(
        brain.status(&s).unwrap().pending,
        0,
        "encrypted notes are never embedded"
    );

    let back = s.move_note_to_vault(n.id, personal).unwrap();
    assert!(!back.encrypted);
    assert_eq!(back.title, "Diary");
    assert_eq!(search(&s, "secret", true).len(), 1);
}

#[test]
fn deleting_encrypted_vault_requires_password() {
    let mut s = store();
    let v = s.create_vault("V", Some("password1")).unwrap();
    let n = note_in(&s, v.id, "", "x");
    assert!(s.delete_vault(v.id, Some("nope-nope")).is_err());
    assert!(s.delete_vault(v.id, None).is_err());
    s.delete_vault(v.id, Some("password1")).unwrap();
    assert!(matches!(s.get_note(n.id), Err(Error::NotFound(..))));
}

// ---------- search ----------

#[test]
fn full_text_search() {
    let s = store();
    let a = note(
        &s,
        "Rust ownership",
        "Borrowing rules and lifetimes explained.",
    );
    note(&s, "Groceries", "milk, eggs, bread");
    let hits = search(&s, "borrow", false);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, a.id);
    assert!(hits[0].snippet.contains(HIGHLIGHT_START) && hits[0].snippet.contains(HIGHLIGHT_END));
    assert_eq!(search(&s, "rust lifetimes", false).len(), 1);
    assert!(search(&s, "\"); DROP TABLE notes; --", false).is_empty());
    s.archive_note(a.id).unwrap();
    assert!(search(&s, "borrow", false).is_empty());
    assert_eq!(search(&s, "borrow", true).len(), 1);
}

// ---------- grammar ----------

#[test]
fn grammar_finds_issues_with_utf16_offsets() {
    let mut g = Grammar::new();
    let text = "🚀 This is an test.";
    let issues = g.check(text);
    let issue = issues
        .iter()
        .find(|i| i.message.to_lowercase().contains("a"))
        .expect("article issue");
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let flagged = String::from_utf16(&utf16[issue.start..issue.end]).unwrap();
    assert_eq!(flagged, "an");
    assert!(g.check("This sentence is fine.").is_empty());
}

// ---------- AI (lexical embedder; no model files needed) ----------

fn ai_corpus(s: &Store) -> (Note, Note, Note) {
    let rust1 = note(
        s,
        "Rust borrow checker",
        "The borrow checker enforces ownership rules in Rust. References must not outlive the data they borrow. Lifetimes describe how long references are valid.",
    );
    let rust2 = note(
        s,
        "Rust lifetimes",
        "Lifetimes in Rust tell the borrow checker how long references live. Ownership and borrowing keep memory safe without a garbage collector.",
    );
    let bake = note(
        s,
        "Sourdough bread",
        "Feed the sourdough starter flour and water. Knead the dough, let it rise overnight, then bake the bread in a hot oven.",
    );
    note(
        s,
        "Pizza dough",
        "Knead flour, water, yeast and salt into a dough. Let the dough rise, stretch it, bake in a very hot oven.",
    );
    note(
        s,
        "Tokio async",
        "Rust async runtimes like Tokio schedule futures. Ownership rules still apply to data moved into async tasks.",
    );
    note(
        s,
        "Banana bread",
        "Mash ripe bananas, mix with flour and sugar, bake the bread for an hour in the oven.",
    );
    (rust1, rust2, bake)
}

#[test]
fn brain_related_and_duplicates() {
    let s = store();
    let (rust1, rust2, bake) = ai_corpus(&s);
    s.create_whiteboard(None, "Board").unwrap();
    let mut brain = Brain::new(None);
    let status = brain.status(&s).unwrap();
    assert!(!status.loaded && !status.neural);
    assert_eq!(status.pending, 6, "whiteboards aren't embedded");

    let related = brain.related(&s, rust1.id, 3).unwrap();
    assert_eq!(related[0].note.id, rust2.id, "{related:?}");
    assert!(!related.iter().take(2).any(|r| r.note.id == bake.id));
    assert_eq!(brain.status(&s).unwrap().pending, 0);

    let copy = note(&s, "Rust borrow checker (copy)", &rust1.body);
    let dups = brain.duplicates(&s, None, None).unwrap();
    assert_eq!(dups.len(), 1);
    let pair = [dups[0].a.id, dups[0].b.id];
    assert!(pair.contains(&rust1.id) && pair.contains(&copy.id));

    s.update_note(
        bake.id,
        NotePatch {
            body: Some("changed".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(brain.status(&s).unwrap().pending, 1);
    s.set_pinned(rust2.id, true).unwrap();
    assert_eq!(brain.index_all(&s).unwrap(), 1, "pinning doesn't re-embed");

    brain.idle_timeout = Duration::ZERO;
    assert!(brain.unload_if_idle());
    assert!(!brain.is_loaded());
}

#[test]
fn brain_is_scoped_to_vault() {
    let mut s = store();
    let (rust1, _, _) = ai_corpus(&s);
    let other = s.create_vault("Other", None).unwrap();
    let twin = note_in(&s, other.id, "Rust borrow checker", &rust1.body);
    let mut brain = Brain::new(None);
    assert!(
        brain.duplicates(&s, None, None).unwrap().is_empty(),
        "twin is in another vault"
    );
    assert!(brain.related(&s, twin.id, 5).unwrap().is_empty());
    assert!(brain.organize(&s, Some(other.id)).unwrap().is_empty());
}

#[test]
fn brain_organize_groups_topics() {
    let s = store();
    let (rust1, rust2, bake) = ai_corpus(&s);
    let mut brain = Brain::new(None);
    let clusters = brain.organize(&s, None).unwrap();
    let find = |id: Uuid| {
        clusters
            .iter()
            .position(|c| c.notes.iter().any(|n| n.id == id))
            .unwrap()
    };
    assert_eq!(find(rust1.id), find(rust2.id), "{clusters:#?}");
    assert_ne!(find(rust1.id), find(bake.id), "{clusters:#?}");
    assert!(clusters.iter().all(|c| !c.label.is_empty()));
}

#[test]
fn brain_suggests_tags() {
    let s = store();
    let (rust1, rust2, _) = ai_corpus(&s);
    s.set_tags(rust2.id, &["rust".into(), "programming".into()])
        .unwrap();
    let mut brain = Brain::new(None);
    let tags = brain.suggest_tags(&s, rust1.id, 6).unwrap();
    let names: Vec<&str> = tags.iter().map(|t| t.tag.as_str()).collect();
    assert!(names.contains(&"rust"), "{tags:?}");
    assert!(names.iter().any(|t| t.contains("borrow")), "{tags:?}");
}

#[test]
fn brain_summarizes() {
    let mut brain = Brain::new(None);
    let text = "Astralis is a notes app built with Rust and Tauri. \
        It stores notes locally and keeps them private. \
        The weather was nice today. \
        Notes can link to each other to form a knowledge graph. \
        Astralis also has tasks, reminders and a calendar for productivity.";
    let s = brain.summarize_text(text, 2).unwrap();
    assert_eq!(s.sentences.len(), 2);
    assert!(!s.text.contains("weather"), "{s:?}");
    let short = brain.summarize_text("Just one sentence here.", 3).unwrap();
    assert_eq!(short.sentences.len(), 1);
}

// ---------- folders ----------

#[test]
fn folders_nest_and_hold_notes() {
    let mut s = store();
    let work = s.create_folder(None, None, "Work").unwrap();
    let projects = s.create_folder(None, Some(work.id), "Projects").unwrap();
    let deep = s.create_folder(None, Some(projects.id), "Apollo").unwrap();
    assert_eq!(deep.parent_id, Some(projects.id));
    assert!(s.create_folder(None, None, "  ").is_err());

    let n = s
        .create_note(NewNote {
            title: "Plan".into(),
            folder_id: Some(deep.id),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(n.folder_id, Some(deep.id));
    let w = s.create_whiteboard(None, "Board").unwrap();
    let w = s.move_note_to_folder(w.id, Some(work.id)).unwrap();
    assert_eq!(w.folder_id, Some(work.id));

    // Sorted by name; renaming works.
    s.rename_folder(work.id, "a-work").unwrap();
    let names: Vec<String> = s
        .list_folders(None)
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(names, vec!["a-work", "Apollo", "Projects"]);

    // No cycles.
    assert!(s.move_folder(work.id, Some(deep.id)).is_err());
    assert!(s.move_folder(work.id, Some(work.id)).is_err());
    s.move_folder(deep.id, None).unwrap();

    // Deleting a folder moves its contents up instead of deleting them.
    s.move_folder(deep.id, Some(projects.id)).unwrap();
    s.delete_folder(projects.id).unwrap();
    assert_eq!(s.get_folder(deep.id).unwrap().parent_id, Some(work.id));
    s.delete_folder(deep.id).unwrap();
    assert_eq!(s.get_note(n.id).unwrap().folder_id, Some(work.id));

    // Folders belong to one vault; moving a note to another vault drops its folder.
    let other = s.create_vault("Other", None).unwrap();
    assert!(s.create_folder(Some(other.id), Some(work.id), "x").is_err());
    assert!(
        s.create_note(NewNote {
            vault_id: Some(other.id),
            folder_id: Some(work.id),
            ..Default::default()
        })
        .is_err()
    );
    assert_eq!(
        s.move_note_to_vault(n.id, other.id).unwrap().folder_id,
        None
    );
    s.delete_vault(other.id, None).unwrap();
}

#[test]
fn folder_names_are_encrypted_in_encrypted_vaults() {
    let mut s = store();
    let v = s.create_vault("Secret", Some("password1")).unwrap();
    let f = s.create_folder(Some(v.id), None, "Medical").unwrap();
    assert_eq!(f.name, "Medical");
    let (name, cipher): (String, Option<Vec<u8>>) = s
        .conn
        .query_row(
            "SELECT name, cipher FROM folders WHERE id = ?1",
            [f.id.as_bytes()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(name.is_empty());
    assert!(!cipher.unwrap().windows(7).any(|w| w == b"Medical"));
    s.rename_folder(f.id, "Health").unwrap();
    assert_eq!(s.list_folders(Some(v.id)).unwrap()[0].name, "Health");
    s.lock_vault(v.id);
    assert!(matches!(
        s.list_folders(Some(v.id)),
        Err(Error::VaultLocked(_))
    ));
}

#[test]
fn grammar_safe_fixes_only_unambiguous() {
    let mut g = Grammar::new();
    let (fixed, n) = g.apply_safe_fixes("This is is a test of the system.");
    assert!(n >= 1, "{fixed}");
    assert_eq!(fixed, "This is a test of the system.");
    let (same, n) = g.apply_safe_fixes("A perfectly fine sentence.");
    assert_eq!((same.as_str(), n), ("A perfectly fine sentence.", 0));
}

// ---------- attachments ----------

#[test]
fn image_attachments_roundtrip_and_follow_notes() {
    use crate::attachments::attachment_ids_in;
    let mut s = store();
    let png = b"\x89PNG\r\n\x1a\nfake-image-bytes".to_vec();
    let a = s
        .add_attachment(None, "cat.png", "image/png", &png)
        .unwrap();
    assert_eq!(a.path, format!("attachments/{}.png", a.id));
    assert_eq!(
        s.get_attachment(a.id).unwrap(),
        ("image/png".to_owned(), png.clone())
    );
    assert!(
        s.add_attachment(None, "x.exe", "application/x-msdownload", &png)
            .is_err()
    );
    assert!(
        s.add_attachment(None, "empty.png", "image/png", &[])
            .is_err()
    );

    let body = format!("Look: ![cat.png|300]({})", a.path);
    assert_eq!(attachment_ids_in(&body), vec![a.id]);
    let n = s
        .create_note(NewNote {
            title: "Cat".into(),
            body,
            ..Default::default()
        })
        .unwrap();

    // Moving the note into an encrypted vault seals its image.
    let v = s.create_vault("Secret", Some("password1")).unwrap();
    s.move_note_to_vault(n.id, v.id).unwrap();
    let (data, encrypted): (Vec<u8>, bool) = s
        .conn
        .query_row(
            "SELECT data, encrypted FROM attachments WHERE id = ?1",
            [a.id.as_bytes()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(encrypted && !data.windows(4).any(|w| w == b"fake"));
    assert_eq!(s.get_attachment(a.id).unwrap().1, png);
    s.lock_vault(v.id);
    assert!(matches!(s.get_attachment(a.id), Err(Error::VaultLocked(_))));
}

// ---------- sync between devices ----------

/// Exchanges changes both ways, like two devices meeting.
fn sync_pair(a: &Store, b: &Store) {
    let ab = a.changes_since(&b.version_vector().unwrap()).unwrap();
    b.apply_changes(&ab).unwrap();
    let ba = b.changes_since(&a.version_vector().unwrap()).unwrap();
    a.apply_changes(&ba).unwrap();
}

#[test]
fn sync_copies_everything_and_is_idempotent() {
    let a = store();
    let b = store();
    assert_ne!(a.device_id(), b.device_id());
    let va = a.default_vault().unwrap();
    let folder = a.create_folder(Some(va), None, "Work").unwrap();
    let sub = a
        .create_folder(Some(va), Some(folder.id), "Projects")
        .unwrap();
    let n = a
        .create_note(NewNote {
            title: "Plan".into(),
            body: "See [[Ideas]]".into(),
            tags: vec!["work".into()],
            folder_id: Some(sub.id),
            vault_id: Some(va),
            ..Default::default()
        })
        .unwrap();
    note_in(&a, va, "Ideas", "loose ideas");
    let p = a
        .create_planner(PlannerInput {
            name: "Launch".into(),
            ..Default::default()
        })
        .unwrap();
    a.create_task(NewTask {
        title: "Ship".into(),
        planner_id: Some(p.id),
        note_id: Some(n.id),
        ..Default::default()
    })
    .unwrap();
    let img = a
        .add_attachment(Some(va), "x.png", "image/png", b"png-bytes")
        .unwrap();

    let cs = a.changes_since(&b.version_vector().unwrap()).unwrap();
    let stats = b.apply_changes(&cs).unwrap();
    assert!(stats.inserted >= 8, "{stats:?}");

    let got = b.get_note(n.id).unwrap();
    assert_eq!(
        (got.title.as_str(), got.body.as_str(), got.folder_id),
        ("Plan", "See [[Ideas]]", Some(sub.id))
    );
    assert_eq!(got.tags, vec!["work"]);
    assert_eq!(b.get_folder(sub.id).unwrap().parent_id, Some(folder.id));
    assert_eq!(
        b.list_tasks(&TaskFilter {
            planner_id: Some(p.id),
            ..Default::default()
        })
        .unwrap()
        .len(),
        1
    );
    assert_eq!(b.get_attachment(img.id).unwrap().1, b"png-bytes");
    // Derived data is rebuilt locally: links resolve and search works on B.
    assert_eq!(b.outgoing_links(n.id).unwrap()[0].notes.len(), 1);
    assert_eq!(b.search(Some(va), "loose", false, 10).unwrap().len(), 1);

    // Re-applying is harmless, and B is now caught up with A.
    let again = b.apply_changes(&cs).unwrap();
    assert_eq!((again.inserted, again.updated), (0, 0));
    assert!(
        a.changes_since(&b.version_vector().unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn sync_merges_concurrent_text_edits() {
    let a = store();
    let b = store();
    let n = a
        .create_note(NewNote {
            title: "Shopping".into(),
            body: "milk\neggs\n".into(),
            ..Default::default()
        })
        .unwrap();
    sync_pair(&a, &b);

    // Both devices edit the same note while apart.
    a.update_note(
        n.id,
        NotePatch {
            body: Some("milk\neggs\nbread\n".into()),
            ..Default::default()
        },
    )
    .unwrap();
    b.update_note(
        n.id,
        NotePatch {
            body: Some("oat milk\neggs\n".into()),
            ..Default::default()
        },
    )
    .unwrap();
    sync_pair(&a, &b);

    let ta = a.get_note(n.id).unwrap().body;
    let tb = b.get_note(n.id).unwrap().body;
    assert_eq!(ta, tb, "both converge");
    assert_eq!(ta, "oat milk\neggs\nbread\n", "and keep both edits");
}

#[test]
fn sync_last_writer_wins_for_fields_and_deletes_propagate() {
    let a = store();
    let b = store();
    let n = a
        .create_note(NewNote {
            title: "Draft".into(),
            ..Default::default()
        })
        .unwrap();
    let gone = a
        .create_note(NewNote {
            title: "Temp".into(),
            ..Default::default()
        })
        .unwrap();
    sync_pair(&a, &b);

    a.update_note(
        n.id,
        NotePatch {
            title: Some("From A".into()),
            ..Default::default()
        },
    )
    .unwrap();
    tick();
    b.update_note(
        n.id,
        NotePatch {
            title: Some("From B".into()),
            ..Default::default()
        },
    )
    .unwrap();
    b.set_pinned(n.id, true).unwrap();
    a.delete_note(gone.id).unwrap();
    sync_pair(&a, &b);

    let (na, nb) = (a.get_note(n.id).unwrap(), b.get_note(n.id).unwrap());
    assert_eq!(na.title, "From B", "later edit wins");
    assert_eq!((na.title, na.pinned), (nb.title, nb.pinned));
    assert!(
        matches!(b.get_note(gone.id), Err(Error::NotFound(..))),
        "delete reached B"
    );
    assert!(
        a.changes_since(&b.version_vector().unwrap())
            .unwrap()
            .is_empty()
    );
    assert!(
        b.changes_since(&a.version_vector().unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn sync_encrypted_vaults_stay_encrypted_in_transit() {
    let mut a = store();
    let mut b = store();
    let v = a.create_vault("Secret", Some("password1")).unwrap();
    let n = note_in(&a, v.id, "Bank", "pin 1234");
    let cs = a.changes_since(&b.version_vector().unwrap()).unwrap();
    let wire = crate::sync::encode(&cs).unwrap();
    assert!(
        !wire.windows(4).any(|w| w == b"1234"),
        "no plaintext in the change set"
    );
    assert!(!String::from_utf8_lossy(&wire).contains("Bank"));
    b.apply_changes(&crate::sync::decode(&wire).unwrap())
        .unwrap();

    assert!(matches!(b.get_note(n.id), Err(Error::VaultLocked(_))));
    b.unlock_vault(v.id, "password1").unwrap();
    assert_eq!(b.get_note(n.id).unwrap().body, "pin 1234");
    b.update_note(
        n.id,
        NotePatch {
            body: Some("pin 9999".into()),
            ..Default::default()
        },
    )
    .unwrap();
    sync_pair(&a, &b);
    assert_eq!(a.get_note(n.id).unwrap().body, "pin 9999");
}

#[test]
fn live_editor_updates_merge_into_notes() {
    let a = store();
    let b = store();
    let n = a.create_note(NewNote { title: "Live".into(), body: "abc".into(), ..Default::default() }).unwrap();
    sync_pair(&a, &b);
    // The editor on A types (its own Yjs client); the update is stored and stamped for sync.
    let base = a.note_crdt(n.id).unwrap();
    let typed = crate::sync::crdt_set_text_for_tests(&base, "abcd", 777);
    assert_eq!(a.apply_note_update(n.id, &typed, true).unwrap().as_deref(), Some("abcd"));
    assert_eq!(a.get_note(n.id).unwrap().body, "abcd");
    // Meanwhile B edited; A's live update folds in without losing B's text.
    b.update_note(n.id, NotePatch { body: Some("xabc".into()), ..Default::default() }).unwrap();
    assert_eq!(b.apply_note_update(n.id, &typed, false).unwrap().as_deref(), Some("xabcd"));
    assert_eq!(b.apply_note_update(n.id, &typed, false).unwrap(), None, "no change the second time");
    // And sync agrees.
    sync_pair(&a, &b);
    assert_eq!(a.get_note(n.id).unwrap().body, "xabcd");
    assert_eq!(b.get_note(n.id).unwrap().body, "xabcd");
}

#[test]
fn sync_merges_concurrent_edits_in_encrypted_notes() {
    let mut a = store();
    let mut b = store();
    let v = a.create_vault("Secret", Some("password1")).unwrap();
    let n = note_in(&a, v.id, "Diary", "one\ntwo\n");
    sync_pair(&a, &b);
    b.unlock_vault(v.id, "password1").unwrap();

    a.update_note(n.id, NotePatch { body: Some("one\ntwo\nthree\n".into()), ..Default::default() }).unwrap();
    b.update_note(n.id, NotePatch { body: Some("zero\none\ntwo\n".into()), ..Default::default() }).unwrap();
    sync_pair(&a, &b);
    assert_eq!(a.get_note(n.id).unwrap().body, "zero\none\ntwo\nthree\n");
    assert_eq!(b.get_note(n.id).unwrap().body, "zero\none\ntwo\nthree\n");
    // The CRDT state is sealed too: no plaintext anywhere in the row.
    let crdt: Vec<u8> = a.conn.query_row("SELECT crdt FROM notes WHERE id = ?1", [n.id.as_bytes()], |r| r.get(0)).unwrap();
    assert!(!crdt.windows(5).any(|w| w == b"three"));
    // Live text works in encrypted vaults too.
    a.update_note(n.id, NotePatch { body: Some("zero\none\ntwo\nthree\nfour\n".into()), ..Default::default() }).unwrap();
    let state = a.note_crdt(n.id).unwrap();
    assert_eq!(b.apply_note_update(n.id, &state, false).unwrap().as_deref(), Some("zero\none\ntwo\nthree\nfour\n"));
}

#[test]
fn sync_merges_whiteboards_by_element() {
    let a = store();
    let b = store();
    let w = a.create_whiteboard(None, "Board").unwrap();
    let el = |id: &str, v: u32, deleted: bool| {
        format!(r#"{{"id":"{id}","type":"rectangle","version":{v},"versionNonce":{v}0,"isDeleted":{deleted}}}"#)
    };
    a.save_whiteboard_scene(w.id, &format!(r#"{{"elements":[{}]}}"#, el("shared", 1, false))).unwrap();
    sync_pair(&a, &b);

    // A draws a box and deletes the shared one; B draws a circle — at the same time.
    a.save_whiteboard_scene(w.id, &format!(r#"{{"elements":[{},{}]}}"#, el("shared", 2, true), el("box", 1, false))).unwrap();
    b.save_whiteboard_scene(w.id, &format!(r#"{{"elements":[{},{}]}}"#, el("shared", 1, false), el("circle", 1, false))).unwrap();
    sync_pair(&a, &b);

    for s in [&a, &b] {
        let scene: serde_json::Value = serde_json::from_str(&s.get_note(w.id).unwrap().body).unwrap();
        let els = scene["elements"].as_array().unwrap();
        let find = |id: &str| els.iter().find(|e| e["id"] == id).cloned();
        assert!(find("box").is_some() && find("circle").is_some(), "both new shapes survive: {scene}");
        assert_eq!(find("shared").unwrap()["isDeleted"], true, "the delete wins over the stale copy");
    }
}

#[test]
fn editor_yjs_updates_apply_in_the_backend() {
    let hex = |s: &str| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect::<Vec<u8>>();
    let s = store();
    let n = s.create_note(NewNote { title: "Compat".into(), body: String::new(), ..Default::default() }).unwrap();
    let base = hex("0101922100040104626f64790b68c3a96c6c6f20f09f9a8000");
    let insert = hex("0101922108849221070620776f726c6400");
    let delete = hex("00019221010001");
    assert_eq!(s.apply_note_update(n.id, &base, true).unwrap().as_deref(), Some("héllo 🚀"));
    assert_eq!(s.apply_note_update(n.id, &insert, true).unwrap().as_deref(), Some("héllo 🚀 world"));
    assert_eq!(s.apply_note_update(n.id, &delete, true).unwrap().as_deref(), Some("éllo 🚀 world"));
    assert_eq!(s.get_note(n.id).unwrap().body, "éllo 🚀 world");
}

// ---- shared vaults ----

/// What the network layer does between two paired devices: each shared vault syncs both ways,
/// each side only accepting rows of vaults it shares with the other.
fn sync_shared(a: &Store, a_id: &str, b: &Store, b_id: &str) {
    for _ in 0..2 {
        for (from, from_id, to, to_id) in [(a, a_id, b, b_id), (b, b_id, a, a_id)] {
            let scope = to.sync_scope(from_id).unwrap();
            for s in from.shares_with(to_id).unwrap() {
                if s.role == ShareRole::Left || !scope.vaults.contains(&s.vault_id) {
                    continue;
                }
                let seen = to.vault_version_vector(s.vault_id).unwrap();
                let cs = from.vault_changes_since(s.vault_id, &seen).unwrap();
                to.apply_vault_changes(&cs, &scope).unwrap();
            }
        }
    }
}

fn vault_names(s: &Store) -> Vec<String> {
    let mut v: Vec<String> = s.list_vaults().unwrap().into_iter().map(|v| v.name).collect();
    v.sort();
    v
}

fn titles(s: &Store, vault: Uuid) -> Vec<String> {
    let mut t: Vec<String> = s
        .list_notes(&ListFilter { vault_id: Some(vault), ..Default::default() })
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect();
    t.sort();
    t
}

#[test]
fn only_shared_vaults_sync_and_show_up_as_their_own_vault() {
    let mut a = store();
    let mut b = store();
    let a_personal = a.default_vault().unwrap();
    let b_personal = b.default_vault().unwrap();
    note_in(&a, a_personal, "A private", "");
    note_in(&b, b_personal, "B private", "");
    let team = a.create_vault("Team", None).unwrap();
    note_in(&a, team.id, "Plan", "ship it");

    // Paired but nothing shared: nothing moves.
    sync_shared(&a, "A", &b, "B");
    assert_eq!(vault_names(&b), vec!["Personal"]);

    // A shares Team; B accepts: it appears next to B's own vault, which is untouched.
    a.share_vault(team.id, "B").unwrap();
    assert!(b.accept_shared_vault(team.id, "A").unwrap());
    sync_shared(&a, "A", &b, "B");
    assert_eq!(vault_names(&b), vec!["Personal", "Team"]);
    assert_eq!(titles(&b, team.id), vec!["Plan"]);
    assert_eq!(titles(&b, b_personal), vec!["B private"]);
    let shared = b.get_vault(team.id).unwrap();
    assert_eq!(shared.owner.as_deref(), Some("A"));
    assert_eq!(a.get_vault(team.id).unwrap().shared_with, vec!["B".to_string()]);

    // Edits flow both ways inside the shared vault — and only there.
    note_in(&b, team.id, "Idea from B", "");
    note_in(&b, b_personal, "Still private", "");
    sync_shared(&a, "A", &b, "B");
    assert_eq!(titles(&a, team.id), vec!["Idea from B", "Plan"]);
    assert_eq!(titles(&a, a_personal), vec!["A private"]);
    assert_eq!(vault_names(&a), vec!["Personal", "Team"]);
}

#[test]
fn deleting_your_own_notes_never_deletes_anyone_elses() {
    let mut a = store();
    let mut b = store();
    let a_personal = a.default_vault().unwrap();
    let b_personal = b.default_vault().unwrap();
    for t in ["one", "two"] {
        note_in(&a, a_personal, t, "");
    }
    let bn = note_in(&b, b_personal, "b note", "");
    let team = a.create_vault("Team", None).unwrap();
    a.share_vault(team.id, "B").unwrap();
    b.accept_shared_vault(team.id, "A").unwrap();
    sync_shared(&a, "A", &b, "B");

    // B deletes everything of its own while apart (even its Personal vault's notes)…
    b.delete_note(bn.id).unwrap();
    sync_shared(&a, "A", &b, "B");
    // …A keeps all of its notes and vaults, and can still write.
    assert_eq!(titles(&a, a_personal), vec!["one", "two"]);
    assert_eq!(vault_names(&a), vec!["Personal", "Team"]);
    note_in(&a, a_personal, "three", "");
    // B can't delete A's shared vault either; it can only leave it.
    assert!(b.delete_vault(team.id, None).is_err());
    let owner = b.leave_vault(team.id).unwrap();
    assert_eq!(owner, "A");
    a.remove_share(team.id, "B").unwrap();
    sync_shared(&a, "A", &b, "B");
    assert_eq!(vault_names(&a), vec!["Personal", "Team"]);
    assert_eq!(vault_names(&b), vec!["Personal"]);
}

#[test]
fn ending_a_share_removes_only_the_members_copy() {
    let mut a = store();
    let mut b = store();
    let team = a.create_vault("Team", None).unwrap();
    note_in(&a, team.id, "Plan", "");
    a.share_vault(team.id, "B").unwrap();
    b.accept_shared_vault(team.id, "A").unwrap();
    sync_shared(&a, "A", &b, "B");
    assert_eq!(titles(&b, team.id), vec!["Plan"]);

    // Unpairing: B loses everything A shared; A keeps it.
    assert!(b.forget_peer("A").unwrap());
    a.forget_peer("B").unwrap();
    assert_eq!(vault_names(&b), vec!["Personal"]);
    assert_eq!(titles(&a, team.id), vec!["Plan"]);
    // B's removal left no tombstones behind that could delete A's notes later.
    let tombs: i64 = b.conn.query_row("SELECT COUNT(*) FROM tombstones", [], |r| r.get(0)).unwrap();
    assert_eq!(tombs, 0);

    // Sharing again sends everything again.
    a.share_vault(team.id, "B").unwrap();
    b.accept_shared_vault(team.id, "A").unwrap();
    sync_shared(&a, "A", &b, "B");
    assert_eq!(titles(&b, team.id), vec!["Plan"]);
    assert_eq!(titles(&a, team.id), vec!["Plan"]);
}

#[test]
fn a_device_cannot_write_outside_what_is_shared_with_it() {
    let mut a = store();
    let mut b = store();
    let a_personal = a.default_vault().unwrap();
    let secret = note_in(&a, a_personal, "Secret", "mine");
    let team = a.create_vault("Team", None).unwrap();
    a.share_vault(team.id, "B").unwrap();
    b.accept_shared_vault(team.id, "A").unwrap();
    sync_shared(&a, "A", &b, "B");

    // B forges rows: an edit of A's private note (claiming it's in Team), a delete of it, and
    // a delete of the whole shared vault (only the owner may do that).
    let mut cs = a.changes_since(&Default::default()).unwrap();
    for r in cs.rows.iter_mut().filter(|r| r.table == "notes") {
        r.values.insert("vault_id".into(), SqlValue::Blob(team.id.as_bytes().to_vec()));
        r.values.insert("body".into(), SqlValue::Text("hacked".into()));
        r.values.insert("hlc".into(), SqlValue::Text("9999999999999.00000.zzz".into()));
    }
    cs.tombstones = vec![
        Tombstone {
            entity: "notes".into(),
            id: secret.id.as_bytes().to_vec(),
            hlc: "9999999999999.00000.zzz".into(),
            origin: "zzz".into(),
            vault_id: Some(team.id.as_bytes().to_vec()),
        },
        Tombstone {
            entity: "vaults".into(),
            id: team.id.as_bytes().to_vec(),
            hlc: "9999999999999.00000.zzz".into(),
            origin: "zzz".into(),
            vault_id: Some(team.id.as_bytes().to_vec()),
        },
    ];
    a.apply_vault_changes(&cs, &a.sync_scope("B").unwrap()).unwrap();
    let n = a.get_note(secret.id).unwrap();
    assert_eq!((n.body.as_str(), n.vault_id), ("mine", a_personal));
    assert!(a.get_vault(team.id).is_ok());
}

#[test]
fn moving_a_note_out_of_a_shared_vault_removes_it_for_members() {
    let mut a = store();
    let mut b = store();
    let a_personal = a.default_vault().unwrap();
    let team = a.create_vault("Team", None).unwrap();
    let n = note_in(&a, team.id, "Draft", "");
    a.share_vault(team.id, "B").unwrap();
    b.accept_shared_vault(team.id, "A").unwrap();
    sync_shared(&a, "A", &b, "B");
    assert_eq!(titles(&b, team.id), vec!["Draft"]);

    a.move_note_to_vault(n.id, a_personal).unwrap();
    sync_shared(&a, "A", &b, "B");
    assert!(titles(&b, team.id).is_empty(), "gone for B");
    assert!(b.get_note(n.id).is_err(), "B never sees A's private vault");
    assert_eq!(titles(&a, a_personal), vec!["Draft"]);
}

#[test]
fn leftover_copies_from_the_old_sync_stay_yours_next_to_the_shared_vault() {
    let mut a = store();
    let mut b = store();
    let shared = a.default_vault().unwrap();
    note_in(&a, shared, "From A", "");
    // Before per-vault sync, pairing copied everything: B has A's "Personal" (same id).
    b.apply_changes(&a.changes_since(&Default::default()).unwrap()).unwrap();
    b.conn.execute_batch(crate::sync::LEGACY_VAULTS_MIGRATION.replace("CREATE TABLE legacy_vaults (id BLOB PRIMARY KEY) WITHOUT ROWID;", "DELETE FROM legacy_vaults;").as_str()).unwrap();
    // Both share "their" Personal with each other — the stuck state.
    a.share_vault(shared, "B").unwrap();
    b.share_vault(shared, "A").unwrap();

    // B's copy also has B's own additions, in a folder, pinned, with a tag.
    let folder = b.create_folder(Some(shared), None, "Mine").unwrap();
    let own = b
        .create_note(NewNote { title: "B only".into(), tags: vec!["b".into()], pinned: true, vault_id: Some(shared), folder_id: Some(folder.id), ..Default::default() })
        .unwrap();
    let b_vaults_before = b.list_vaults().unwrap().len();

    // Only the device that didn't create it gives way: its copy becomes its own vault.
    assert!(!a.accept_shared_vault(shared, "B").unwrap());
    assert!(b.accept_shared_vault(shared, "A").unwrap());
    sync_shared(&a, "A", &b, "B");
    assert_eq!(b.list_vaults().unwrap().len(), b_vaults_before + 1, "both listed");
    assert_eq!(b.get_vault(shared).unwrap().owner.as_deref(), Some("A"));
    assert_eq!(titles(&b, shared), vec!["From A"], "A's vault as A has it");
    let mine = b.list_vaults().unwrap().into_iter().find(|v| v.owner.is_none() && titles(&b, v.id).contains(&"B only".to_string())).unwrap();
    assert_eq!(mine.name, "Personal");
    assert_eq!(titles(&b, mine.id), vec!["B only", "From A"], "B's copy kept, with its own notes");
    let copied = b.list_notes(&ListFilter { vault_id: Some(mine.id), ..Default::default() }).unwrap().into_iter().find(|n| n.title == "B only").unwrap();
    assert!(copied.pinned && copied.tags == vec!["b".to_string()] && copied.folder_id.is_some());
    assert!(b.get_note(own.id).is_err() || b.get_note(own.id).unwrap().vault_id != shared);

    // A stops sharing: only the shared vault goes; B's own copy stays.
    a.unshare_vault(shared, "B").unwrap();
    b.drop_shared_vault(shared).unwrap();
    assert!(b.get_vault(shared).is_err());
    assert_eq!(titles(&b, mine.id), vec!["B only", "From A"]);
    assert_eq!(titles(&a, shared), vec!["From A"], "A's untouched");

    // Vaults made since can never be taken over that way.
    let mine = b.create_vault("Mine", None).unwrap();
    assert!(!b.accept_shared_vault(mine.id, "A").unwrap());
    assert!(b.get_vault(mine.id).unwrap().owner.is_none());
}
