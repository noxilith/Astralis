---
name: astralis-brain
description: How to use the Astralis Brain (astralis-brain MCP tools), this project's long-term memory. Load this skill on its own as your very first step (no file reads in the same step) at the start of every task or question about a project (getting oriented, "how do I…", "where is…", fixing, building). The Brain often already has the answer. Also use it whenever you learn something worth remembering, and before ending a session with unfinished work.
---

# Astralis Brain

The Brain is long-term memory shared by every session and every AI tool the user works with (Claude Code, Codex, Cursor…). It exists to **save tokens**: a small, targeted answer from the Brain replaces re-reading files, grepping, and re-asking the user. Treat it like a sharp colleague's notebook: check it first, keep it accurate, and keep it short.

Tools (from the `astralis-brain` MCP server): `brain_context`, `brain_recall`, `brain_read`, `brain_map`, `brain_remember`, `brain_update`, `brain_forget`, `brain_map_note`, `brain_handoff`, and for navigating code without reading whole files, `code_outline` and `code_find`.

## The rule: the Brain, never the old chats

Never search or read earlier chat logs, session transcripts or history files (for example `~/.claude/projects/*.jsonl`) to find out what was decided, learned or done before. That is slow, costs far more tokens than the Brain, and is exactly what it replaces. Ask `brain_recall`. If the Brain doesn't know, read the code or ask the user, then **save the answer** with `brain_remember` so nobody has to dig again. Save as you go (when a decision is made, a problem is solved, a command is found), not only at the end: a session can stop at any time.

If the user has set up the Brain's hooks, the index arrives by itself at the start of a session and clearly relevant memories are added to messages (as "Brain — remembered from earlier"). Use them, and still `brain_recall` for anything else.

## Where we left off

Each project has one **hand-off note**. The index shows it right under the header ("Where we left off"), so a new session knows what was in progress and what comes next without reading the last chat. Keep it current: before you stop, or when you reach a natural pause, call `brain_handoff` with a few lines (what's done, what's half-done, the next step, anything to watch out for). It replaces the previous note; don't append history to it.

## 1. Start: load the index once

**Make `brain_context` your very first tool call, on its own** (unless the index is already in your context from a hook), and pass `task`, one sentence about what you're about to do, so the relevant memories come first. Don't read files, grep or list directories in the same step. Look at the index first, then decide what (if anything) you still need to read; often the answer is already there.

If the Brain's tools aren't loaded yet (some clients defer MCP tools), load them before doing anything else. In Claude Code that's one tool search for `select:mcp__astralis-brain__brain_context,mcp__astralis-brain__brain_recall,mcp__astralis-brain__brain_map,mcp__astralis-brain__brain_remember,mcp__astralis-brain__brain_map_note`. Don't read files while you wait.

`brain_context` returns one line per memory, grouped by type, with a `[handle]`. It also lists the user's preferences that apply everywhere ("Me") and says whether a code map exists.

- Memories nobody has used for two months drop out of this list (they are still found by `brain_recall`).
- Don't call it again in the same task. It doesn't change unless you save something.
- Read it fully. It's short on purpose. Note any memory that bears on the task, and follow "Me" preferences, such as how the user likes explanations or which commands to avoid.

## 2. Before you read or search, ask the Brain

Before opening files, grepping, re-reading a long chat, or asking the user something you might already know:

- **Where is something?** Use `brain_map` with a plain-words query ("where are sync conflicts resolved"). You get only the matching files with one-line purposes. A file marked **changed since described** or **gone** must be re-checked. The rest you can trust enough to open directly, without searching.
- **How or why is something done?** Use `brain_recall` with a plain-words question. It searches by meaning and returns the best few memories, each trimmed to its most relevant lines.
  - Phrase it the way you'd ask a teammate: "how do we run the tests", "why is sync per vault", "what breaks the Linux build".
  - Keep `limit` small (the default of 4 is usually right).
- **Need the whole memory?** Use `brain_read [handle]`, but only when the trimmed lines aren't enough.

If the Brain doesn't know, fall back to files and search as usual. Then save what you found (step 3), so the next session doesn't pay for it again.

Memories can be stale. Use them to go straight to the right place. Verify before anything destructive or irreversible: deleting, migrating, publishing.

## Code: outline and find instead of reading whole files

- `code_outline path` returns just the definitions in a file (functions, classes, headings…) with line numbers and signatures, a few hundred tokens instead of the whole file. Read the outline first, then read only the lines you need.
- `code_find name` says where a function, type or class is defined, anywhere in the project. Use it before grepping.

## 3. Save what's worth knowing next time

When you save, the Brain may answer that a **similar memory** already exists: if yours replaces or contradicts it, `brain_update` that memory (or `brain_forget` the wrong one) instead of leaving both. Details are capped at about 1,500 characters; one idea per memory, so split anything bigger.

Use `brain_remember` when you learn something durable that would cost tokens or time to rediscover:

| type | save | example hook |
|---|---|---|
| `decision` | what was decided **and why** | "Sync is per vault, so peers can't write outside shared vaults (security)" |
| `gotcha` | a trap that wasted time | "Linking fails without CC=gcc on this machine; run.sh sets it" |
| `howto` | commands or steps you'll need again | "Run tests: ./run.sh test (needs webkit paths from run.sh)" |
| `fact` | a non-obvious truth about the project | "Notes and whiteboards share the notes table (kind column)" |
| `preference` | how the user likes things done | "Wants plain-language explanations, no jargon" |
| `context` | where unfinished work stands | "Drive sync: invite flow done; member-to-member P2P not started" |

How to write a good memory:

- **One idea per memory.** Two facts means two calls.
- **The hook states the conclusion, not the topic.** Good: "Tests must run serially: they share one SQLite file". Bad: "Testing notes".
- **Details are optional and short:** the reason, the exact command, the file path. No essays, and no code that's already in the repo (point to the file instead).
- **Choose the scope.** Use `scope: "me"` only for preferences that apply to *every* project. Everything else belongs to the project (the default).
- **Saving the same point twice is safe.** The Brain updates the existing memory instead of adding a duplicate. Still, prefer `brain_update` when you know the handle.

Don't save:

- secrets, tokens, passwords or personal data
- anything obvious from the code, the README or `git log`
- temporary state (a failing test you're about to fix, a branch name for today)
- guesses you haven't confirmed

## 4. Build the code map as you go

Once you've understood what a file does, record it with `brain_map_note`: the path relative to the project root and one line of purpose. Batch several files in one call.

- Describe what the file is *for*, not every function in it: "Per-vault sync: offers, scope checks, purge".
- When a flagged ("changed since described") file has been re-read, note it again to refresh it.
- If a file was deleted or its purpose moved, give an empty purpose to remove it.

You don't need to map the whole repository. Map the files you actually needed; that's exactly what the next session will need too.

## 5. Keep it true

- If a memory turns out wrong, `brain_update` it right away (new hook or details).
- If it's no longer true at all, `brain_forget` it. It goes to the archive and can be restored.
- If what you did changes a decision or how-to, update that memory rather than adding a contradicting one.

## 6. Before you finish

If work is unfinished, or the next session needs to know where things stand, save or update **one** `context` memory for the project. Say what's done, what's next and anything blocking, in two or three lines. Keep a single current context memory: update it instead of adding a new one each session.

## 7. The user's own notes

The same server reaches the user's notes app: `notes_search`, `notes_list`, `notes_read`, `notes_create`, `notes_edit`, `notes_move`, `notes_delete`, and folders with `notes_folders` / `notes_folder_create` / `notes_folder_rename` / `notes_folder_delete`. Use them **only when the user asks** for something about their notes. They aren't the place for project memory; that's what the Brain is for.

- Find the note first (`notes_search` or `notes_list`), then `notes_read` it before editing.
- Edit small: `old_text` + `new_text` for a passage, `append` to add at the end. Rewrite the whole `body` only when asked to rewrite the note.
- Keep the user's structure, wording and Markdown; change only what they asked for.
- `notes_delete` archives the note; it can be restored. Delete it for good (`permanent: true`, on an archived note) only when the user clearly asked for that. Every edit keeps the previous version in the note's history.
- `notes_move` files a note or a whole folder into another folder, out to the top level (`to_folder: "/"`), or into another vault. Deleting a folder keeps what's in it (it moves up a level).
- If a tool says the user allows reading only, or no access, say so and suggest the change instead.

## 8. The user's tasks and calendar

Also **only when the user asks**: `plan_overview` (planners with progress, plus today's date and time; call it first), `planner_create` / `planner_update` / `planner_delete`, `tasks_list` / `tasks_add` / `tasks_edit` / `tasks_delete`, and `calendar_list` / `calendar_add` / `calendar_edit` / `calendar_delete`.

- Planning a project (an app, a trip): break it into concrete, ordered tasks and make them in one `planner_create` call, with priorities and due dates the user gave. Don't invent deadlines.
- Times are the user's local time: `YYYY-MM-DD` (all day) or `YYYY-MM-DD HH:MM`. Work out "tomorrow" or "next Tuesday" from the date `plan_overview` gives.
- Appointments and work sessions go on the calendar (`calendar_add`; a start time alone makes it an hour, add `remind_minutes` for a reminder). Things to do go in tasks.
- Look before you change: `tasks_list` / `calendar_list` give the `[handle]`s. Tick a task off with `tasks_edit` `status: "done"` (a repeating task moves to its next date) rather than deleting it.
- If a tool says the user allows reading only, or no access, say so and suggest the change instead.

## Token discipline, in short

- One `brain_context` per task, then targeted `brain_recall` / `brain_map` lookups.
- Ask narrow questions and keep limits small; use `brain_read` only when needed.
- Trust the map's unflagged entries to skip searching; re-read only flagged files.
- Save findings right away while they're fresh, in a line or two each.
