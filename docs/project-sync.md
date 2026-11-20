# Projects in your cloud drive

Move your code between computers through your own Google Drive or OneDrive, the same way cloud
vaults work. On a new computer (or a fresh Linux install): install Astralis, connect the drive,
open **Projects**, and restore each project into a folder.

## How it works

- **Snapshot.** When your AI tool saves a hand-off (`brain_handoff`), the Brain adds the project to
  Projects (if it is not there) and takes a snapshot of it: a git bundle of the whole repository (every branch and tag, so
  restoring needs no network and no git login) plus one extra commit holding your uncommitted work.
  It is built with a throwaway git index, so your own index and files are never touched.
- **Upload.** Astralis (while it is open) seals the snapshot and uploads it to
  `Astralis/Projects/` on your drive. The key lives on the drive's top-level Astralis folder, the
  same trust as a cloud vault's key, so a new device that connects the same drive can open it. File
  names are hashes: the drive does not see project names. The newest two snapshots are kept.
- **Catching up is yours to do.** Nothing is downloaded on its own, and your AI tool never pulls. When
  another device has newer work, the project shows *Cloud is newer* and an **Update from cloud**
  button. It applies only when nothing here is unsaved, you are on the same branch, and this device
  has no commits the cloud copy lacks; otherwise it changes nothing and says why. Until you update,
  this device's hand-offs are not uploaded, so they can't bury the newer work.
- **Restoring.** Projects → *Restore…* sets the project up in a new or empty folder with its history,
  its `origin` URL (credentials removed), and the unsaved work that was in the snapshot.

## What is left out

- Anything git ignores (`.env`, build output, `node_modules`…) is never included. That also means
  secrets in ignored files do **not** travel: copy them yourself.
- A built-in list of generated folders (`node_modules/`, `target/`, `dist/`, `build/`, caches…) and
  whatever is in the project's `.astralisignore` (same syntax as `.gitignore`) are left out of the
  uncommitted work. Your AI tool is told to write one when a project's sync is first turned on.
- Committed files are part of the repository's history and always travel with it, so
  `.astralisignore` can't shrink those.

## Switching it on and off

On by default: a project your AI tool works in is added to Projects and uploaded at each hand-off
(once a drive is connected). Turn off *Keep every project my AI tools work in* so new projects
aren't kept, untick *Sync* on a single project, or *Add* a folder by hand.

## Limits

- Git repositories only, with at least one commit.
- The whole repository is uploaded each time something changes, so a repository with a very large
  history costs a lot of upload each hand-off. Incremental bundles would fix that.
- Two devices that both commit between syncs diverge; nothing is overwritten, the cloud copy is
  kept as `refs/astralis/in/<branch>` for you to merge.

## The Projects tab

Projects works like Notes: the sidebar lists the selected project's files (what git tracks plus new
files that aren't ignored), you open any file to see it with syntax colors, and Ctrl S saves it.
Cloud sync is the **Sync** tab on the right.

### Run button

The Run button at the top right starts the project. Your AI tool writes what to run, once per
project, in one of these (it is told so at the start of a session):

- `.astralis/run.json`: `{"name": "Dev server", "command": "npm run dev"}`
- `.astralis/run.sh`: a shell script (run with `sh`, or `bash` on Windows)

The command runs in the project folder and its output shows under the editor; Stop ends it and
everything it started. It is not an interactive terminal (no typing into it).

**Safety:** the run file is part of the synced project, so a command can arrive from another device.
Astralis asks you to confirm the exact command the first time, and again whenever the command or a
script it names (like `./dev.sh`) changes. A script that script calls, or a program the command
starts, is not re-checked.
