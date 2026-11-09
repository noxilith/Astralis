//! Tauri commands: a thin layer that exposes `astralis-core` to the frontend.
//! Quick database calls run synchronously; AI, grammar and password work runs off the main thread.

use astralis_core::ai::{
    BrainStatus, Cluster, DuplicatePair, RelatedNote, Summary, TagSuggestion, TidyResult,
};
use astralis_core::attachments::Attachment;
use astralis_core::calendar::{CalendarItem, Event, EventInput};
use astralis_core::folders::Folder;
use astralis_core::grammar::{Grammar, GrammarIssue};
use astralis_core::history::NoteVersion;
use astralis_core::links::{Backlink, Graph, OutgoingLink};
use astralis_core::merge::MergeRequest;
use astralis_core::notes::{ListFilter, NewNote, Note, NotePatch, NoteRef, TagCount};
use astralis_core::planners::{Planner, PlannerInput};
use astralis_core::search::SearchHit;
use astralis_core::tasks::{NewTask, ReminderKind, Task, TaskFilter, TaskPatch};
use astralis_core::templates::{Template, TemplateInput};
use astralis_core::vault::Vault;
use astralis_core::{Error, Result, Timestamp};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use tauri::State;
use uuid::Uuid;

use std::sync::Arc;

use crate::SharedState;

type S<'a> = State<'a, SharedState>;

/// Runs blocking work (AI, grammar, Argon2) on a worker thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| Error::Ai(format!("worker failed: {e}")))?
}

macro_rules! store_cmd {
    ($name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty => |$s:ident| $body:expr) => {
        #[tauri::command]
        pub fn $name(state: S<'_>, $($arg: $ty),*) -> Result<$ret> {
            #[allow(unused_mut)]
            let mut $s = state.store.lock().unwrap();
            $body
        }
    };
}

/// Like `store_cmd!`, but async on a worker thread (for slow work like key derivation).
macro_rules! slow_store_cmd {
    ($name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty => |$s:ident| $body:expr) => {
        #[tauri::command]
        pub async fn $name(state: S<'_>, $($arg: $ty),*) -> Result<$ret> {
            let st = state.inner().clone();
            blocking(move || {
                #[allow(unused_mut)]
                let mut $s = st.store.lock().unwrap();
                $body
            })
            .await
        }
    };
}

// ---- notes & whiteboards ----
store_cmd!(create_note(input: NewNote) -> Note => |s| s.create_note(input));
store_cmd!(quick_capture(vault_id: Option<Uuid>, text: String) -> Note => |s| s.quick_capture(vault_id, &text));
store_cmd!(get_note(id: Uuid) -> Note => |s| s.get_note(id));
store_cmd!(update_note(id: Uuid, patch: NotePatch) -> Note => |s| s.update_note(id, patch));
store_cmd!(list_notes(filter: ListFilter) -> Vec<Note> => |s| s.list_notes(&filter));
store_cmd!(set_pinned(id: Uuid, pinned: bool) -> Note => |s| s.set_pinned(id, pinned));
store_cmd!(archive_note(id: Uuid) -> Note => |s| s.archive_note(id));
store_cmd!(unarchive_note(id: Uuid) -> Note => |s| s.unarchive_note(id));
store_cmd!(delete_note(id: Uuid) -> () => |s| s.delete_note(id));
store_cmd!(set_tags(id: Uuid, tags: Vec<String>) -> Note => |s| s.set_tags(id, &tags));
store_cmd!(add_tag(id: Uuid, tag: String) -> Note => |s| s.add_tag(id, &tag));
store_cmd!(remove_tag(id: Uuid, tag: String) -> Note => |s| s.remove_tag(id, &tag));
store_cmd!(list_tags(vault_id: Option<Uuid>) -> Vec<TagCount> => |s| s.list_tags(vault_id));
store_cmd!(merge_notes(request: MergeRequest) -> Note => |s| s.merge_notes(request));
store_cmd!(search(vault_id: Option<Uuid>, query: String, include_archived: bool) -> Vec<SearchHit>
    => |s| s.search(vault_id, &query, include_archived, 50));
store_cmd!(create_whiteboard(vault_id: Option<Uuid>, title: String) -> Note => |s| s.create_whiteboard(vault_id, &title));
store_cmd!(save_whiteboard(id: Uuid, scene: String) -> () => |s| s.save_whiteboard_scene(id, &scene));

// ---- image attachments (bytes travel as base64) ----
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentData {
    mime: String,
    data: String,
}

store_cmd!(add_attachment(vault_id: Option<Uuid>, name: String, mime: String, data: String) -> Attachment => |s| {
    let bytes = B64.decode(data.as_bytes()).map_err(|e| Error::Invalid(format!("bad image data: {e}")))?;
    s.add_attachment(vault_id, &name, &mime, &bytes)
});
store_cmd!(get_attachment(id: Uuid) -> AttachmentData => |s| {
    let (mime, bytes) = s.get_attachment(id)?;
    Ok(AttachmentData { mime, data: B64.encode(bytes) })
});

// ---- folders ----
store_cmd!(list_folders(vault_id: Option<Uuid>) -> Vec<Folder> => |s| s.list_folders(vault_id));
store_cmd!(create_folder(vault_id: Option<Uuid>, parent_id: Option<Uuid>, name: String) -> Folder
    => |s| s.create_folder(vault_id, parent_id, &name));
store_cmd!(rename_folder(id: Uuid, name: String) -> Folder => |s| s.rename_folder(id, &name));
store_cmd!(move_folder(id: Uuid, parent_id: Option<Uuid>) -> Folder => |s| s.move_folder(id, parent_id));
store_cmd!(delete_folder(id: Uuid) -> () => |s| s.delete_folder(id));
store_cmd!(move_note_to_folder(note_id: Uuid, folder_id: Option<Uuid>) -> Note => |s| s.move_note_to_folder(note_id, folder_id));

// ---- links & graph ----
store_cmd!(backlinks(id: Uuid) -> Vec<Backlink> => |s| s.backlinks(id));
store_cmd!(outgoing_links(id: Uuid) -> Vec<OutgoingLink> => |s| s.outgoing_links(id));
store_cmd!(find_by_title(vault_id: Uuid, title: String) -> Vec<NoteRef> => |s| s.find_by_title(vault_id, &title));
store_cmd!(suggest_titles(vault_id: Option<Uuid>, query: String) -> Vec<NoteRef> => |s| s.suggest_titles(vault_id, &query, 12));
store_cmd!(graph(vault_id: Option<Uuid>, include_orphans: bool) -> Graph => |s| s.graph(vault_id, include_orphans));

// ---- history ----
store_cmd!(note_history(id: Uuid) -> Vec<NoteVersion> => |s| s.note_history(id));
store_cmd!(snapshot_note(id: Uuid) -> () => |s| s.snapshot_note(id));
store_cmd!(restore_version(version_id: Uuid) -> Note => |s| s.restore_version(version_id));

// ---- templates ----
store_cmd!(list_templates() -> Vec<Template> => |s| s.list_templates());
store_cmd!(create_template(input: TemplateInput) -> Template => |s| s.create_template(input));
store_cmd!(update_template(id: Uuid, input: TemplateInput) -> Template => |s| s.update_template(id, input));
store_cmd!(delete_template(id: Uuid) -> () => |s| s.delete_template(id));
store_cmd!(template_from_note(note_id: Uuid, name: String) -> Template => |s| s.template_from_note(note_id, &name));
store_cmd!(note_from_template(template_id: Uuid, title: Option<String>, vault_id: Option<Uuid>) -> Note
    => |s| s.note_from_template(template_id, title, vault_id));

// ---- tasks & reminders ----
store_cmd!(create_task(input: NewTask) -> Task => |s| s.create_task(input));
store_cmd!(update_task(id: Uuid, patch: TaskPatch) -> Task => |s| s.update_task(id, patch));
store_cmd!(complete_task(id: Uuid) -> Task => |s| s.complete_task(id));
store_cmd!(reopen_task(id: Uuid) -> Task => |s| s.reopen_task(id));
store_cmd!(delete_task(id: Uuid) -> () => |s| s.delete_task(id));
store_cmd!(list_tasks(filter: TaskFilter) -> Vec<Task> => |s| s.list_tasks(&filter));
store_cmd!(snooze_reminder(kind: ReminderKind, id: Uuid, until: Timestamp) -> () => |s| s.snooze_reminder(kind, id, until));

// ---- calendar ----
store_cmd!(create_event(input: EventInput) -> Event => |s| s.create_event(input));
store_cmd!(update_event(id: Uuid, input: EventInput) -> Event => |s| s.update_event(id, input));
store_cmd!(delete_event(id: Uuid) -> () => |s| s.delete_event(id));
store_cmd!(get_event(id: Uuid) -> Event => |s| s.get_event(id));
store_cmd!(calendar(start: Timestamp, end: Timestamp) -> Vec<CalendarItem> => |s| s.calendar(start, end));

// ---- planners ----
store_cmd!(list_planners() -> Vec<Planner> => |s| s.list_planners());
store_cmd!(create_planner(input: PlannerInput) -> Planner => |s| s.create_planner(input));
store_cmd!(update_planner(id: Uuid, input: PlannerInput) -> Planner => |s| s.update_planner(id, input));
store_cmd!(delete_planner(id: Uuid) -> () => |s| s.delete_planner(id));

// ---- vaults ----
store_cmd!(list_vaults() -> Vec<Vault> => |s| s.list_vaults());
store_cmd!(default_vault() -> Uuid => |s| s.default_vault());
store_cmd!(rename_vault(id: Uuid, name: String) -> Vault => |s| s.rename_vault(id, &name));
store_cmd!(lock_vault(id: Uuid) -> () => |s| { s.lock_vault(id); Ok(()) });
store_cmd!(lock_all_vaults() -> () => |s| { s.lock_all_vaults(); Ok(()) });
store_cmd!(move_note_to_vault(note_id: Uuid, vault_id: Uuid) -> Note => |s| s.move_note_to_vault(note_id, vault_id));
// Argon2id is deliberately slow, so password operations run off the main thread.
slow_store_cmd!(create_vault(name: String, password: Option<String>) -> Vault
    => |s| s.create_vault(&name, password.as_deref()));
slow_store_cmd!(unlock_vault(id: Uuid, password: String) -> Vault => |s| s.unlock_vault(id, &password));
slow_store_cmd!(change_vault_password(id: Uuid, old: String, new: String) -> ()
    => |s| s.change_vault_password(id, &old, &new));
/// Deletes one of our vaults; devices it was shared with drop their copies.
#[tauri::command]
pub async fn delete_vault(state: S<'_>, id: Uuid, password: Option<String>) -> Result<()> {
    let st = state.inner().clone();
    let members = blocking(move || {
        let mut s = st.store.lock().unwrap();
        let members: Vec<String> = s.vault_shares(id)?.into_iter().map(|x| x.peer).collect();
        s.delete_vault(id, password.as_deref())?;
        Ok(members)
    })
    .await?;
    notify_shares(&state, &members);
    Ok(())
}

// ---- shared vaults ----

fn notify_shares(state: &SharedState, peers: &[String]) {
    if let Some(net) = state.net.get() {
        for p in peers {
            net.shares_changed(p);
        }
    }
}

/// Shares one of our vaults with a paired device; it shows up there as its own vault.
#[tauri::command]
pub fn vault_share(state: S<'_>, vault_id: Uuid, peer: String) -> Result<Vault> {
    let v = {
        let s = state.store.lock().unwrap();
        s.share_vault(vault_id, &peer)?;
        s.get_vault(vault_id)?
    };
    notify_shares(&state, &[peer]);
    Ok(v)
}

/// Stops sharing a vault with a device (its copy is removed there).
#[tauri::command]
pub fn vault_unshare(state: S<'_>, vault_id: Uuid, peer: String) -> Result<Vault> {
    let v = {
        let s = state.store.lock().unwrap();
        s.unshare_vault(vault_id, &peer)?;
        s.get_vault(vault_id)?
    };
    notify_shares(&state, &[peer]);
    Ok(v)
}

/// Leaves a vault someone shared with us: our copy is removed, theirs is untouched.
#[tauri::command]
pub fn vault_leave(state: S<'_>, vault_id: Uuid) -> Result<()> {
    let owner = state.store.lock().unwrap().leave_vault(vault_id)?;
    notify_shares(&state, &[owner]);
    Ok(())
}

// ---- settings ----
store_cmd!(get_setting(key: String) -> Option<String> => |s| s.get_setting(&key));
store_cmd!(set_setting(key: String, value: String) -> () => |s| s.set_setting(&key, &value));

// ---- grammar (rule-based, not AI) ----
#[tauri::command]
pub async fn check_grammar(state: S<'_>, text: String) -> Result<Vec<GrammarIssue>> {
    let st = state.inner().clone();
    blocking(move || {
        let mut g = st.grammar.lock().unwrap();
        Ok(g.get_or_insert_with(Grammar::new).check(&text))
    })
    .await
}

// ---- the Brain (in-app AI) ----
macro_rules! brain_cmd {
    ($name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty => |$s:ident, $b:ident| $body:expr) => {
        #[tauri::command]
        pub async fn $name(state: S<'_>, $($arg: $ty),*) -> Result<$ret> {
            let st = state.inner().clone();
            blocking(move || {
                let $s = st.store.lock().unwrap();
                #[allow(unused_mut)]
                let mut $b = st.brain.lock().unwrap();
                $body
            })
            .await
        }
    };
}

brain_cmd!(brain_status() -> BrainStatus => |s, b| b.status(&s));
brain_cmd!(related_notes(id: Uuid) -> Vec<RelatedNote> => |s, b| b.related(&s, id, 8));
brain_cmd!(find_duplicates(vault_id: Option<Uuid>, threshold: Option<f32>) -> Vec<DuplicatePair>
    => |s, b| b.duplicates(&s, vault_id, threshold));
brain_cmd!(organize_notes(vault_id: Option<Uuid>) -> Vec<Cluster> => |s, b| b.organize(&s, vault_id));
brain_cmd!(suggest_tags(id: Uuid) -> Vec<TagSuggestion> => |s, b| b.suggest_tags(&s, id, 6));
brain_cmd!(summarize_note(id: Uuid, sentences: Option<usize>) -> Summary
    => |s, b| b.summarize_note(&s, id, sentences.unwrap_or(3)));

/// "Tidy up": formats the note, organizes long unstructured text into sections, and (optionally)
/// applies unambiguous spelling/grammar fixes. Returns a preview; nothing is saved here.
#[tauri::command]
pub async fn tidy_note(state: S<'_>, text: String, fix_spelling: bool) -> Result<TidyResult> {
    let st = state.inner().clone();
    blocking(move || {
        let body = text;
        let mut result = st.brain.lock().unwrap().tidy(&body)?;
        if fix_spelling {
            let mut g = st.grammar.lock().unwrap();
            let (fixed, n) = g
                .get_or_insert_with(Grammar::new)
                .apply_safe_fixes(&result.body);
            if n > 0 {
                result.body = fixed;
                result.changes.push(if n == 1 {
                    "Fixed 1 spelling or grammar mistake".into()
                } else {
                    format!("Fixed {n} spelling or grammar mistakes")
                });
            }
        }
        Ok(result)
    })
    .await
}

pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        create_note,
        quick_capture,
        get_note,
        update_note,
        list_notes,
        set_pinned,
        archive_note,
        unarchive_note,
        delete_note,
        set_tags,
        add_tag,
        remove_tag,
        list_tags,
        merge_notes,
        search,
        create_whiteboard,
        save_whiteboard,
        add_attachment,
        get_attachment,
        list_folders,
        create_folder,
        rename_folder,
        move_folder,
        delete_folder,
        move_note_to_folder,
        backlinks,
        outgoing_links,
        find_by_title,
        suggest_titles,
        graph,
        note_history,
        snapshot_note,
        restore_version,
        list_templates,
        create_template,
        update_template,
        delete_template,
        template_from_note,
        note_from_template,
        create_task,
        update_task,
        complete_task,
        reopen_task,
        delete_task,
        list_tasks,
        snooze_reminder,
        create_event,
        update_event,
        delete_event,
        get_event,
        calendar,
        list_planners,
        create_planner,
        update_planner,
        delete_planner,
        list_vaults,
        default_vault,
        create_vault,
        rename_vault,
        unlock_vault,
        lock_vault,
        lock_all_vaults,
        change_vault_password,
        delete_vault,
        vault_share,
        vault_unshare,
        vault_leave,
        crate::cloud::google_status,
        crate::cloud::google_set_client,
        crate::cloud::google_connect,
        crate::cloud::google_disconnect,
        crate::cloud::cloud_create_vault,
        crate::cloud::cloud_invites,
        crate::cloud::cloud_invite,
        crate::cloud::cloud_uninvite,
        crate::cloud::cloud_join,
        crate::cloud::cloud_grant_folder,
        crate::cloud::cloud_sync_now,
        crate::cloud::open_url,
        crate::assistant_cmds::assistant_status,
        crate::assistant_cmds::assistant_download,
        crate::assistant_cmds::assistant_chat,
        crate::assistant_cmds::assistant_cancel,
        crate::assistant_cmds::assistant_unload,
        crate::update::update_check,
        crate::update::update_install,
        crate::brain_cmds::brain_projects,
        crate::brain_cmds::brain_memories,
        crate::brain_cmds::brain_set_reviewed,
        crate::brain_cmds::brain_forget,
        crate::brain_cmds::brain_tidy,
        crate::brain_cmds::brain_merge,
        crate::brain_cmds::brain_compact,
        crate::brain_cmds::brain_stats,
        crate::brain_cmds::brain_graph,
        crate::brain_cmds::brain_setup,
        crate::brain_cmds::brain_set_mirror,
        crate::brain_cmds::brain_skill_status,
        crate::brain_cmds::brain_install_skill,
        crate::brain_cmds::brain_skill_text,
        move_note_to_vault,
        get_setting,
        set_setting,
        check_grammar,
        brain_status,
        related_notes,
        find_duplicates,
        organize_notes,
        suggest_tags,
        summarize_note,
        tidy_note,
        p2p_status,
        p2p_set_name,
        p2p_set_color,
        p2p_start_pairing,
        p2p_cancel_pairing,
        p2p_join,
        p2p_unpair,
        note_crdt,
        note_edit,
        live_awareness,
        live_presence,
        live_scene,
    ]
}

// ---- peer-to-peer ----

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pStatus {
    ready: bool,
    id: String,
    name: String,
    /// This device's color as collaborators see it.
    color: String,
    peers: Vec<astralis_net::PeerStatus>,
}

fn net_of(state: &SharedState) -> Result<Arc<astralis_net::Net>> {
    state.net.get().cloned().ok_or_else(|| Error::Invalid("peer-to-peer is still starting".into()))
}

fn net_err(e: impl std::fmt::Display) -> Error {
    Error::Invalid(e.to_string())
}

#[tauri::command]
pub fn p2p_status(state: S<'_>) -> Result<P2pStatus> {
    Ok(match state.net.get() {
        Some(net) => P2pStatus { ready: true, color: net.color(), id: net.id(), name: net.name(), peers: net.peers() },
        None => P2pStatus { ready: false, color: String::new(), id: String::new(), name: String::new(), peers: vec![] },
    })
}

#[tauri::command]
pub fn p2p_set_name(state: S<'_>, name: String) -> Result<()> {
    net_of(&state)?.set_name(&name).map_err(net_err)
}

/// The color collaborators see for this device.
#[tauri::command]
pub fn p2p_set_color(state: S<'_>, color: String) -> Result<()> {
    net_of(&state)?.set_color(&color).map_err(net_err)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    /// 6-digit code (for devices on the same network).
    code: String,
    /// Code plus this device's identity (works from anywhere).
    invite: String,
}

#[tauri::command]
pub fn p2p_start_pairing(state: S<'_>) -> Result<Pairing> {
    let net = net_of(&state)?;
    let code = net.start_pairing();
    Ok(Pairing { invite: net.invite(&code), code })
}

#[tauri::command]
pub fn p2p_cancel_pairing(state: S<'_>) -> Result<()> {
    net_of(&state)?.cancel_pairing();
    Ok(())
}

#[tauri::command]
pub async fn p2p_join(state: S<'_>, code: String) -> Result<astralis_net::PeerStatus> {
    let net = net_of(state.inner())?;
    net.join(&code, None).await.map_err(net_err)
}

#[tauri::command]
pub fn p2p_unpair(state: S<'_>, id: String) -> Result<()> {
    net_of(&state)?.unpair(&id).map_err(net_err)
}

/// A note's text CRDT (a Yjs update) for the editor to load.
#[tauri::command]
pub fn note_crdt(state: S<'_>, note_id: Uuid) -> Result<Vec<u8>> {
    state.store.lock().unwrap().note_crdt(note_id)
}

/// An edit typed in the editor (a Yjs update): store it, and send it to connected devices.
/// Returns the note's new text if it changed.
#[tauri::command]
pub fn note_edit(state: S<'_>, note_id: Uuid, update: Vec<u8>) -> Result<Option<String>> {
    let body = state.store.lock().unwrap().apply_note_update(note_id, &update, true)?;
    if let Some(net) = state.net.get().filter(|n| n.connected_count() > 0) {
        net.broadcast(astralis_net::Live::Update { note_id: note_id.to_string(), update });
    }
    Ok(body)
}

/// Names and carets in a note (a Yjs awareness update) for connected devices.
#[tauri::command]
pub fn live_awareness(state: S<'_>, note_id: Uuid, update: Vec<u8>) -> Result<()> {
    if let Some(net) = state.net.get().filter(|n| n.connected_count() > 0) {
        net.broadcast(astralis_net::Live::Awareness { note_id: note_id.to_string(), update });
    }
    Ok(())
}

/// Where the user is: caret/selection in a note, or pointer on a whiteboard.
#[tauri::command]
pub fn live_presence(state: S<'_>, note_id: Option<Uuid>, anchor: Option<u32>, head: Option<u32>, pointer: Option<[f64; 2]>) -> Result<()> {
    if let Some(net) = state.net.get().filter(|n| n.connected_count() > 0) {
        net.broadcast(astralis_net::Live::Presence { note_id: note_id.map(|u| u.to_string()), anchor, head, pointer });
    }
    Ok(())
}

/// Live whiteboard drawing: the changed elements.
#[tauri::command]
pub fn live_scene(state: S<'_>, note_id: Uuid, elements: String) -> Result<()> {
    if let Some(net) = state.net.get().filter(|n| n.connected_count() > 0) {
        net.broadcast(astralis_net::Live::Scene { note_id: note_id.to_string(), elements });
    }
    Ok(())
}
