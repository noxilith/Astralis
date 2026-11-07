# Astralis

A lightweight, private, AI-powered notes and productivity app. Rust + Tauri 2 + React.

## Quick start

```bash
sudo pacman -S --needed webkit2gtk-4.1 base-devel   # Linux system deps for Tauri (Arch)
./run.sh fetch-model                                # optional: neural AI model (~90 MB)
./run.sh app                                        # launch the desktop app
```

| Command | What it does |
|---|---|
| `./run.sh app` | Desktop app in dev mode (hot reload) |
| `./run.sh test` | Core test suite |
| `./run.sh ui` | Frontend only, in a browser, with an in-memory mock backend |
| `./run.sh build` | Release installers |
| `./run.sh fetch-model` | Download the Brain's embedding model |
| `./run.sh check` / `clippy` / `fmt` | Rust tooling |

## Layout

```
crates/astralis-core/     All logic, UI-agnostic (SQLite, crypto, AI). Tested.
  src/notes.rs            Notes, quick capture, pin, archive, tags
  src/links.rs            [[Wiki links]], backlinks, rename-aware link rewriting, graph
  src/history.rs          Automatic versions (≤ every 5 min), restore
  src/merge.rs            Merge notes (originals archived, links redirected)
  src/templates.rs        Templates with {{date}}/{{title}}/… placeholders
  src/tasks.rs            Tasks: priorities, due dates, recurrence, reminders
  src/calendar.rs         Events + combined calendar (incl. projected recurring tasks)
  src/planners.rs         Project boards (to do / doing / done)
  src/whiteboards.rs      Whiteboards — a kind of note whose body is an Excalidraw scene
  src/folders.rs          Nested folders inside a vault (names encrypted in encrypted vaults)
  src/vault.rs            Vaults (workspaces), optionally encrypted (Argon2id + XChaCha20-Poly1305)
  src/search.rs           Full-text search (FTS5)
  src/grammar.rs          Offline grammar/spelling (Harper, rule-based)
  src/sync/               Sync foundation: clocks, change sets, CRDT text merging
  src/ai/                 On-device AI (see below)
apps/desktop/             Tauri 2 app
  src-tauri/src/          Commands (thin wrappers over the core), reminder + indexing loops
  src/                    React + TypeScript UI
```

## The Brain (in-app AI)

Runs entirely in-process; notes never leave the device. One small sentence-embedding model
(all-MiniLM-L6-v2 via [candle](https://github.com/huggingface/candle), CPU) turns each note
into a vector, stored next to the note. Everything else is math on those vectors:

- **Related notes / duplicates** — nearest neighbours / near-identical pairs
- **Smart organization** — spherical k-means clustering, groups named by TF-IDF key terms
- **Auto-tagging** — tags from similar notes + key phrases ranked by semantic closeness
- **Summaries** — extractive (centrality + MMR), so they never invent facts

The model loads on first use and unloads after 2 idle minutes. Without the model files the
Brain falls back to a built-in lexical embedder, so every feature works offline out of the box.

## Vaults

Notes live in vaults — separate workspaces switched from the dropdown above the note list.
Inside a vault, notes can be organized into folders nested to any depth (drag to move;
right-click a folder for actions). Search, tags, links, the graph, the archive and the Brain
all work within the current vault;
tasks, calendar and planners are shared. Notes and whiteboards are both kinds of note.

A vault can be **encrypted** with a password. Its notes are encrypted at rest and never indexed,
searched, tagged, linked, versioned or embedded. Moving a note into an encrypted vault erases
all of that derived data. Unlocked keys live only in memory. Cloud vaults are planned.

## Sync foundation

Every change to a synced row (vaults, folders, notes, tasks, events, planners, templates,
images) is stamped by SQLite triggers with a hybrid logical clock and the device it came from;
deletes leave tombstones. Devices exchange *change sets* — everything newer than what the other
has seen — and applying them is idempotent and order-independent. Fields merge last-writer-wins;
note text merges as a CRDT (yrs) so concurrent edits on two devices both survive. Encrypted
vaults travel as ciphertext. Search, links, history and AI data are rebuilt on each device.
Transport (P2P, cloud drives) is the next layer.

## Privacy

- Local SQLite database in the app data folder; nothing is sent anywhere.
- The AI runs on-device and never sees notes in encrypted vaults.

## Not built yet

Planned for later passes: sync (custom CRDT, P2P), cloud providers (Drive, OneDrive, Box, Dropbox, pCloud),
end-to-end encrypted collaboration, biometric unlock, voice-to-text, a fine-tuned Astralis model.
