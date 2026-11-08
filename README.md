# Astralis

A private, local-first notes and productivity app with a built-in AI that runs on your own
computer. Rust + Tauri 2 + React.

- **Notes, whiteboards and folders** in separate vaults, optionally encrypted
- **Tasks, calendar and project boards**
- **Ask Astralis**: a brutally honest AI to talk to about the note you're on, run locally
- **The Brain**: project memory for AI agents (Claude Code, Codex, Cursor…) so they spend fewer tokens
- **Live collaboration and sync**: peer-to-peer between devices, shared vaults, Google Drive
- **A 3D graph** of your notes: every vault a galaxy, linked notes form solar systems

Nothing leaves your device unless you sync it, and synced data is end-to-end encrypted.

## Build and run

### Requirements

- Rust (stable) and Node.js 20+
- Tauri's system packages (on Linux, webkit2gtk-4.1)
- A C/C++ toolchain and CMake (for llama.cpp, which runs the built-in AI)
- Vulkan headers and `glslc` (the GPU build of llama.cpp). On macOS Metal is used instead, so
  these aren't needed.

| System | Install |
|---|---|
| Arch | `sudo pacman -S --needed webkit2gtk-4.1 base-devel cmake vulkan-headers shaderc spirv-headers` |
| Debian / Ubuntu | `sudo apt install libwebkit2gtk-4.1-dev build-essential cmake libvulkan-dev glslc spirv-headers` |
| macOS | Xcode command line tools and `brew install cmake` |
| Windows | Visual Studio Build Tools (C++), CMake, and the [Vulkan SDK](https://vulkan.lunarg.com/) |

If SPIRV-Headers isn't installed, `run.sh` fetches it into `.deps/` on the first build.

### Quick start

```bash
./run.sh fetch-model   # optional: the note-embedding model (~90 MB)
./run.sh app           # build and launch the desktop app
```

The first build compiles llama.cpp and takes several minutes. `run.sh` limits parallel jobs so
the build fits in memory on a laptop; set `CARGO_BUILD_JOBS` higher if you have RAM to spare.

| Command | What it does |
|---|---|
| `./run.sh app` | Desktop app in dev mode (hot reload) |
| `./run.sh second [profile]` | A second window with its own profile, to try sync and collaboration on one machine |
| `./run.sh ui` | Frontend only, in a browser, with an in-memory mock backend |
| `./run.sh test` | Rust test suite |
| `./run.sh build` | Release installers |
| `./run.sh fetch-model` | Download the note-embedding model |
| `./run.sh seed-test-vault [profile]` | A 1,000-note "Test galaxy" vault, for the graph |
| `./run.sh seed-test-brain [--fresh] [profile]` | 1,000 test memories in the Brain |
| `./run.sh check` / `clippy` / `fmt` | Rust tooling |

## Features

### Notes and vaults

Notes live in **vaults**, separate workspaces switched from the dropdown above the note list,
with folders nested to any depth. Notes have tags, `[[wiki links]]` with backlinks,
automatic version history, templates, merging, full-text search and offline grammar and
spelling checks. **Whiteboards** (Excalidraw) are a kind of note.

A vault can be **encrypted** with a password (Argon2id + XChaCha20-Poly1305). Its notes are
encrypted at rest and never indexed, searched, linked, versioned or embedded, and unlocked keys
live only in memory.

### Ask Astralis

Open a note's side panel and switch to **Chat** to talk to a local language model (Microsoft's
Phi-3 mini, MIT, a one-time 2.4 GB download) about the note:

- **Be brutally honest / Will it pay off?** A verdict on the idea, what's wrong with it, and
  whether it's worth the time and money. Every answer is honest about the payoff.
- **Give me ideas.** Ideas that fit what you ask, each with its competition and a realistic
  payoff, and a pick.
- **Make a plan / Write it up.** A concrete plan, or the whole note rewritten. Replies can
  replace the note or be added to it, and it's undoable.

It runs on [llama.cpp](https://github.com/ggml-org/llama.cpp) on the CPU. GPU support
(Vulkan, or Metal on macOS) is built in but off until it's been tested on more hardware; set
`ASTRALIS_LLM_GPU=1` to try it. The model loads on your first message and is freed when you
close the panel or after 10 idle minutes.

### The Brain: memory for AI agents

The Brain gives coding agents a memory of your projects, so they don't re-read chats and files
to get back up to speed. It's a vault of short Markdown memories (decisions, conventions,
gotchas, a map of the code), one section per project plus one about you, that agents read and
write through an [MCP](https://modelcontextprotocol.io) server, `astralis-mcp`. You review what
they save in the app, and it's mirrored to a plain Markdown folder you can open in any editor.

Set it up from **Brain → Connect** in the app. It shows the config for your agent and installs
the [`astralis-brain` skill](skills/astralis-brain/SKILL.md), which teaches the agent to use it
well. The Brain view also has a 3D graph of memories in the shape of a brain.

### Related notes, tags and summaries

A small sentence-embedding model (all-MiniLM-L6-v2 via
[candle](https://github.com/huggingface/candle)) turns each note into a vector. That powers
related notes and duplicate detection, smart grouping, auto-tagging, and extractive summaries
(they only quote your note, so they never invent facts). Without the model files these fall
back to a built-in lexical embedder, so they work offline out of the box.

### Sync and collaboration

- **Peer-to-peer** between your devices (iroh: encrypted QUIC, found on the local network or
  anywhere through public relays). Pair with a 6-digit code in **Settings → Devices**.
- **Live editing** while connected: text streams as you type (Yjs), and each person's caret or
  whiteboard pointer shows with their name and colour.
- **Shared vaults**: share one vault with a paired device. It appears as its own vault on theirs
  and never touches their other vaults; stopping sharing removes it from their device.
- **Google Drive**: keep a vault on your Drive and invite people by email. Everything stored
  on Drive is end-to-end encrypted, and the app can only see the files it created. Members of
  a Drive vault also sync with each other peer-to-peer. To build with Google sign-in you need
  your own OAuth client; see [docs/google-drive-setup.md](docs/google-drive-setup.md).

Under the hood every change is stamped with a hybrid logical clock; devices exchange change
sets that apply in any order, fields merge last-writer-wins, and note text merges as a CRDT
(yrs) so concurrent edits both survive.

### The graph

Every vault is a 3D galaxy you can fly around. Notes linked together form **solar systems**:
the most-linked note is the sun, notes linked to it are planets, and notes reached through a
planet are its moons. Unlinked notes are lone stars along the spiral arms. Drag to turn,
Ctrl + scroll to zoom, click a galaxy or system to fly there, click a planet to open it.

## Layout

```
crates/astralis-core/     All app logic, UI-agnostic (SQLite, crypto, sync, AI). Tested.
  src/ai/                 Embeddings (candle), the local language model (llama.cpp), Ask Astralis
  src/memory.rs           The Brain: AI memories, recall, code map, tidy, Markdown mirror
  src/sync/               Clocks, change sets, CRDT text merging, vault sharing
  src/vault.rs            Vaults, encryption
crates/astralis-net/      Peer-to-peer: iroh endpoint, pairing, sync, live editing
crates/astralis-cloud/    Google sign-in, Drive client, end-to-end encrypted cloud sync
crates/astralis-mcp/      The Brain's MCP server for AI agents
apps/desktop/             Tauri 2 app
  src-tauri/src/          Commands (thin wrappers over the core), background loops
  src/                    React + TypeScript UI
skills/astralis-brain/    The skill that teaches AI agents to use the Brain
docs/                     Setup guides
```

## Privacy

- Your notes live in a local SQLite database in the app's data folder.
- The AI features run on your device and never see notes in encrypted vaults.
- Sync is peer-to-peer and encrypted; anything stored on Google Drive is end-to-end encrypted.
- The only downloads are the AI models, from Hugging Face, when you ask for them.

## Planned

More cloud drives (OneDrive, Dropbox, Box, pCloud), biometric unlock, voice-to-text, and a
fine-tuned Astralis model.

## License

Astralis is free software, licensed under the
[GNU General Public License v3.0 or later](LICENSE).
