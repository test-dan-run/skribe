# Skribe: implementation plan

This plan turns the requirements in [AGENTS.md](AGENTS.md) into an ordered build. AGENTS.md
remains the source of requirements; where this plan and AGENTS.md disagree, AGENTS.md wins and
this plan must be corrected.

**Status (2026-10-14):** Phases 0 and 1 are implemented, except that the CI workflow has not yet
run on a hosted Windows runner. Later phases are not started. Section 3.8 records what the first
two phases established, and each later phase in section 4 lists what it must take note of.

## 1. Starting point

- Before Phase 0 the repository contained only `AGENTS.md`: no source, lockfile, CI workflow,
  design document or test, and no commits on `main`.
- AGENTS.md is written as if a codebase already exists. Three of its instructions referred to
  things that were not in this repository:
  - "Retain this stack, its lockfiles and module boundaries": Phase 0 created them.
  - "Preserve established rules for tied events, zero and end boundaries, and the initial
    boundary": still not recorded anywhere; Phase 2 records them (D6).
  - "Retain ... one compiler/CMake job and the documented hosted-runner resource limits":
    Phase 0 created the workflow and `docs/ci.md`; the limits await review (D5).
- The owner confirmed on 2026-10-13 that no earlier code, design or CI exists. The project is
  greenfield, and this plan supplies the missing rules itself (section 2 and section 3.5).

## 2. Decisions

### 2.1 Decided (2026-10-13)

| # | Decision | Outcome |
|---|----------|---------|
| D1 | Product name and Tauri bundle identifier | Product "Skribe", identifier `com.skribe.app`. The identifier fixes the app data location (storage identity) and must not change after the first distributed build. AGENTS.md was renamed from "Scribe" to match |
| D2 | JavaScript package manager | npm; `package-lock.json` is the only JavaScript lockfile |
| D4 | Meaning of "one compiler/CMake job" | One sequential CI job on a Windows runner runs all checks and performs the only release compile (Rust, bundled C such as SQLite, and any CMake-built dependency). CI does not build inference runtimes from source |
| D5 | Hosted-runner resource limits | Proposed in `docs/ci.md` during Phase 0 (runner size, job timeout, build parallelism, disk budget) for the owner's review; that file is then "the documented limits". Written but not yet reviewed, and not yet exercised by a run |

### 2.2 Open

Each row has a recommendation the plan assumes. The "needed by" phase is the last point at which
changing the answer is cheap.

| # | Decision | Recommendation | Needed by |
|---|----------|----------------|-----------|
| D3 | Rust layout | Cargo workspace: thin Tauri app crate plus library crates added as phases need them (section 3.1). Phase 0 was built this way; confirmation is still open | Phase 0 |
| D6 | Timeline tie and boundary rules | Adopt the rules in section 3.5 | Phase 2 |
| D7 | Library location | Default under the user's local app data; user-selectable folder later, moved only by an explicit, verified migration. Phase 1 uses the default location only; confirmation is still open, and stays cheap until the first distributed build | Phase 1 |
| D8 | Inference integration | Runtimes are separately installed native executables run as child processes by Rust (section 3.7), not code linked into the app | Phase 6 |
| D9 | Engines | Whisper-family speech recognition via a whisper.cpp runtime; ONNX-based segmentation and speaker embedding for diarisation; a llama.cpp runtime with a GGUF instruct model for minutes. Confirm with the spike in Phase 6 | Phase 6 |
| D10 | Where packages are hosted and who builds them | Reviewed catalogue pins upstream release archives by URL, size and SHA-256 where upstream publishes suitable Windows builds; otherwise a separate packaging workflow publishes them | Phase 6 |
| D11 | Catalogue trust | Catalogue shipped inside the app, plus a user-initiated refresh that is accepted only with a valid signature from a key embedded in the app | Phase 6 |
| D12 | First-release languages and hardware floor | English first; CPU-only path must work; GPU acceleration is optional and catalogue-gated | Phase 6 |
| D13 | Behaviour when a capture device disappears mid-recording | Keep the healthy track recording, mark the lost track failed from that timestamp, do not switch devices automatically | Phase 3 |
| D14 | Reprocessing and manual edits | A new run is a separate result set; the previous run and its edits stay intact; the user explicitly chooses the active run. No automatic transfer of text edits between runs | Phase 8 |

## 3. Architecture

### 3.1 Repository layout

```
AGENTS.md
IMPLEMENTATION_PLAN.md
package.json, package-lock.json, vite.config.ts, tsconfig.json
Cargo.toml, Cargo.lock            workspace root
src/                              React + TypeScript
  app/                            shell, navigation, error boundary
  features/{library,meetings,prepare,record,review,minutes,models,settings}/
                                  per feature: presentational components + controller hooks
  ipc/                            generated command and event types, one typed wrapper
  ui/                             shared accessible primitives
src-tauri/                        Tauri app crate: wiring, commands, capabilities, shortcut,
                                  audio URI scheme, NSIS configuration
crates/
  skribe-domain/                  IDs, states, timeline derivation, staleness. No I/O
  skribe-store/                   SQLite schema, migrations, repositories, library, backup
  skribe-ipc/                     command names, payload types, typed errors, command handlers;
                                  no Tauri dependency. Its tests generate src/ipc/generated
  skribe-audio/                   capture trait, WASAPI backend, track writers, recovery
  skribe-models/                  catalogue, download, verification, install, removal
  skribe-pipeline/                job runner, stages, checkpoints, runtime supervision
catalogue/                        reviewed catalogue metadata and its schema
docs/                             ci.md, storage.md, timeline-rules.md, decisions/
scripts/                          generate-ipc.mjs, ci/package-manifests.mjs
.github/workflows/ci.yml
```

Boundaries, enforced by crate dependencies and review:

- React components render state and raise intents. Controller hooks call `src/ipc` only. No
  component calls Tauri APIs directly.
- `src-tauri` commands validate input, call a service, and map errors. No business logic.
- `skribe-domain` depends on nothing in the workspace and performs no I/O, so the timeline and
  state rules are testable on Linux and Windows alike.
- Only `skribe-store` touches SQLite. Only `skribe-audio` touches audio devices. Only
  `skribe-pipeline` starts runtime processes. Only `skribe-models` uses the network.
- Windows-only code sits behind `cfg(windows)`. WSL builds compile everything else. A synthetic
  capture backend exists for tests and development builds only; release builds never include it,
  and a non-Windows build states that capture is unavailable.

Command and event types are generated from Rust into `src/ipc` so the two sides cannot drift.
`ts-rs` generates them from the types in `skribe-ipc` (`tauri-specta` is still a release
candidate). The same crate's tests fail when the committed TypeScript is out of date. The Tauri
app crate has no tests: on Windows a test binary that links the webview needs extra manifest
handling, so everything testable lives in the library crates.

### 3.2 Storage layout

Three roots, never nested inside each other or inside the install directory:

```
<library>/                    meeting data; owned by one library ID
  library.json                library ID, format version, created-by app version
  library.lock                held while one process has the library open
  db/skribe.sqlite            structured data (WAL)
  db/migration-in-progress.json   present only while a migration runs
  db/set-aside-<time>/        a database that a restore replaced, kept as found
  audio/<meeting>/<track>.wav original tracks + <track>.journal + <track>.clock
  derived/<meeting>/          prepared audio, playback mix, pipeline checkpoints (sensitive)
  backups/                    verified database snapshots, skribe-v<schema>-<time>.sqlite
                              (no audio, no models)
  trash/                      recoverable deleted meetings
<models>/                     outside the library and outside the app binaries
  staging/                    partial downloads and unvalidated extractions
  packages/<id>/<version>/    validated, registered packages
<temp>/                       scratch; safe to clear on startup
```

The database stores paths relative to their root. Application update and default uninstall
leave `<library>` and `<models>` untouched. [docs/storage.md](docs/storage.md) describes what is
implemented, including the migration and restore procedures.

### 3.3 Data model

Stable IDs (UUIDv7, generated in Rust) and foreign keys throughout. Editable rows carry an
integer `revision` used for optimistic concurrency. Times are integer microseconds from the
recording's monotonic origin.

| Area | Tables | Notes |
|------|--------|-------|
| Meetings | `meetings`, `agenda_items` | Three separate state columns: recording, processing, minutes |
| Audio | `tracks`, `track_health_events` | Source (microphone or system), device identity, native format, start offset, frame count, SHA-256, health, finalisation result |
| Timeline | `sections`, `timeline_events`, `timeline_overrides` | Events are append-only with a per-meeting sequence number; review edits are overrides, never updates to events |
| Notes | `notes`, `note_revisions` | Original timestamp immutable; unique retry key per note |
| Processing | `jobs`, `job_stages`, `processing_runs` | A run records exact model and runtime versions; old runs are retained |
| Transcript | `transcript_segments`, `segment_overrides`, `speakers`, `speaker_labels` | Segments are immutable model output; overrides hold user text and labels |
| Minutes | `minutes_sections`, `minutes_revisions`, `minutes_sources` | Each revision stores its origin (generated or manual), a source fingerprint and its links to segments and notes |
| Packages | `installed_packages`, `package_selections` | Registered only after validation |
| Other | `settings`, FTS5 indexes over effective transcript text and notes | |

### 3.4 Lifecycle states

Recording, processing and minutes are separate state machines. The conceptual flow in AGENTS.md
is a view derived from the three.

- Recording: `setup → ready → recording → finalising → recorded`, plus `interrupted` and `failed`.
- Processing: `none → waiting_for_models → queued → running → review`, plus `cancelled`,
  `failed`, `interrupted`.
- Minutes: `none → generating → current → stale`, plus `candidate_available`, `cancelled`, `failed`.

Rules:

- Processing can be queued only when recording is `recorded` and both tracks finalised successfully.
- A track that failed is reported as failed. The other track's audio, the notes and the events
  are kept. A meeting with one failed track is `recorded` with a per-track health warning only if
  the failed track finalised a valid partial file; otherwise recording is `failed` and the
  healthy data remains reviewable.
- On startup, any meeting in `recording` or `finalising` with no live session becomes
  `interrupted` and is offered recovery. Any job in `running` becomes `interrupted` and can
  resume from its last checkpoint.
- There is no pause. The state machine has no pause transition, and the UI has no pause control.

### 3.5 Timeline rules (proposed; see D6)

To be recorded in `docs/timeline-rules.md` and locked with tests once confirmed.

1. The effective timeline is a pure function of `timeline_events` plus `timeline_overrides`.
   Section membership is never stored; it is derived.
2. Sections are half-open intervals `[start, end)`. A note or segment whose timestamp equals a
   boundary belongs to the section that starts there.
3. A transcript segment belongs to the section containing its start time.
4. Initial boundary: the first section starts at time zero. It can be renamed but not moved or
   removed. With no agenda, it is an additional section with a default title.
5. Zero boundary: no other boundary can be placed at or before zero.
6. End boundary: no boundary can be placed at or after the recording end. Items stamped exactly
   at the end belong to the last section.
7. Ties: events with equal timestamps are ordered by persisted sequence number. Two transitions
   at the same instant leave the earlier section present and empty; nothing is dropped.
8. Shortcut: each trigger advances to the next unvisited agenda item. Once the agenda is
   exhausted, each trigger creates a new persistent additional section titled
   "Additional section N". It never wraps and is never ignored.
9. Agenda items never reached remain in the structure as unvisited sections with no time range.
10. Rename changes the title only. Insert, move and remove change boundaries, and therefore
    membership and minutes staleness, in one transaction.

### 3.6 Command surface

- Narrow, typed commands per use case (`agenda_reorder`, `note_create`, `boundary_move`), each
  taking IDs and expected revisions. No generic query, file, URL or process command.
- Rust validates ID format and existence, revisions, text length, list sizes and state
  preconditions, and returns a typed error with a code, a user-facing explanation, what data was
  retained and what recovery is available.
- The frontend receives no filesystem, shell, HTTP or opener capability. Audio reaches the
  player through a custom URI scheme handled in Rust that accepts only a meeting ID and track ID
  and supports range requests.
- Progress and state changes are pushed as typed events. Polling, where used, must not overwrite
  an error the user has not yet acted on. No event exists yet (section 3.8).
- Section 3.8 gives the concrete steps for adding a command in the code as built.

### 3.7 Inference process model (see D8)

- A runtime package is a validated directory of native executables and libraries. Rust starts
  the executable from that directory with arguments built only from catalogue metadata and
  internal paths. The frontend can select a catalogue entry; it cannot supply arguments or paths.
- Communication is over standard streams or a private pipe. No runtime listens on a network
  port unless the spike shows there is no alternative, in which case it binds to loopback only
  with a per-launch secret.
- One runtime process at a time. Cancellation terminates the process; the stage resumes from its
  last checkpoint.
- Runtime output, prompts and checkpoints are meeting content. They go to `derived/`, not to
  application logs. Logs record stage names, durations, counts and error codes only.

### 3.8 Established in Phases 0 and 1

Later phases build on the following. Each point is enforced by a test, a lint rule or the
compiler unless it says otherwise.

Adding a command:

1. Add the name to `crates/skribe-ipc/src/command_names.rs`. `src-tauri/build.rs` includes that
   file and turns each name into a permission.
2. Write the handler and its argument and result types in `skribe-ipc`, and add the command to
   `command_specs` in `bindings.rs`. A command takes no parameter or exactly one named `args`,
   and returns `Result<T, CommandError>`.
3. Add a thin adapter in `src-tauri/src/commands.rs` and list it in `generate_handler!` in
   `src-tauri/src/lib.rs`. Nothing checks this step: a missing adapter fails only when the
   command is called.
4. Grant `allow-<command-name>` in `src-tauri/capabilities/default.json`.
5. Run `npm run ipc:generate` and commit `src/ipc/generated`.

The Rust tests fail when steps 1, 2, 4 and 5 disagree.

Errors:

- Storage failures are `StoreError` variants. `crates/skribe-ipc/src/error.rs` maps each one to
  a `CommandError`: code, message, what was retained, recovery text and the recoveries the app
  can perform. The mapping is an exhaustive match, so a new variant does not compile until it
  is described. Add it to `every_store_error` in that file's tests by hand.
- In the frontend, `call()` in `src/ipc` returns a `CommandResult` and never throws. A
  controller keeps the original failure on screen when a recovery attempt fails.

Where code and tests go:

- The Tauri app crate has no tests and `cargo test` does not build it. Logic behind anything
  registered there (global shortcut, audio URI scheme, window-close handling) belongs in a
  library crate, where it is tested without a webview.
- A new crate joins `members` in the root `Cargo.toml` and sets `[lints] workspace = true`. The
  workspace forbids `unsafe`; a crate that needs it overrides that lint for itself only.
- ESLint allows `@tauri-apps/*` imports only in `src/ipc`, and lets `.tsx` components import
  IPC types but not `call`. Commands are called from controller hooks in `.ts` files.

Capabilities, events and Content Security Policy:

- The window is granted exactly the app's commands. A test asserts that the capability file
  lists those and nothing else: no event, window, file, shell, HTTP or opener permission.
- No typed event exists yet. `library_status` waits for the open attempt to finish instead.
  The first phase that pushes events grants the event-listen permission, changes that test
  deliberately, and generates the payload types in `skribe-ipc`.
- The policy loads scripts, styles, images, fonts and media only from the app bundle and
  connects only to Tauri's IPC. Inline scripts and injected `<style>` elements are blocked,
  which rules out CSS-in-JS libraries. Any new origin, such as the audio URI scheme, needs its
  own entry.
- The policy is attached to content served from the app bundle. Under `tauri dev` the window
  loads the Vite server directly, so the policy is not in force there (read from Tauri's
  source, not run). A new UI dependency or origin is checked in a built app: `tauri build`,
  with or without `--debug`.

Database:

- Schema 1 holds only `library_meta`. Each phase adds the tables it first needs (section 3.3)
  as a new SQL file in `crates/skribe-store/migrations/`, appended to `MIGRATIONS` in
  `migrate.rs`. Shipped entries are never edited. All pending migrations run in one transaction
  with a foreign-key check before commit.
- One worker thread owns the connection and runs work serially. `Database::call` is private to
  `skribe-store`; other crates use repository methods added there. Each call is one short
  transaction, because a long one blocks every other command.
- SQLite integers are signed 64-bit and `rusqlite` does not bind `u64`. Timeline microseconds,
  wall-clock milliseconds and sizes are stored as `i64`.
- The bundled SQLite is compiled with FTS5 and JSON1.
- Wall-clock times cross the command boundary as Unix milliseconds and are formatted in the
  frontend. The workspace has no date-time crate.
- The app process exits without dropping the open library. The database is safe (replaying a
  leftover write-ahead log is tested), but nothing may rely on destructors running at exit.

Library behaviour that later data changes:

- Once `audio/`, `derived/`, `backups/` or `trash/` holds anything, a missing database or
  missing `library.json` stops the app with a recovery screen instead of starting fresh.
- Nothing removes snapshots or `set-aside-*` folders, and nothing clears `<temp>` yet.
- A second app instance is told the library is in use. There is no single-instance handoff.

Frontend:

- Confirmations are inline disclosures that open with focus on Cancel
  (`src/features/library/LibraryUnavailable.tsx`). jsdom has no `<dialog>.showModal()`, so a
  modal dialog would be untestable without a polyfill.
- `src/ui/` does not exist yet. Shared primitives are extracted when a second feature needs one.
- TypeScript is held at 6.0 because `typescript-eslint` does not support 7.
  `eslint-plugin-jsx-a11y` is absent because it does not support ESLint 10.

Verification limits:

- WSL cannot build the installer or compile `cfg(windows)` code. For those, CI is the only
  check before a Windows machine is used.

## 4. Phases

Each phase ends with working, tested behaviour and a green CI run that still produces an
installer. Phases 3, 6 and 7 start with a short spike because they carry the most uncertainty.

### Phase 0: Scaffold and CI

- Scaffold Tauri 2 + React + TypeScript + Vite with current stable versions, pinned by committed
  lockfiles. Create the workspace with `skribe-domain` and `skribe-store` as empty crates.
- Tooling: `rustfmt`, `clippy` with warnings as errors, ESLint, strict `tsc`, Vitest, type
  generation for `src/ipc`.
- Capabilities file granting only what the shell needs. Content Security Policy that blocks
  remote content.
- CI workflow (section 6) producing an NSIS installer artifact from the first commit.
- `docs/ci.md` (D5) and `docs/storage.md`. Set the product name and identifier from D1.
- Exit: `npm run check` and `cargo test --workspace` pass in WSL; CI is green on a Windows
  runner and uploads installer, checksums and manifests; the installed app opens to an honest
  empty state.

Status: implemented, with the CI exit criterion still open. The empty state was checked in a
Linux build of the app, not in an installed Windows build. Remaining:

- Push and get the first green run. The workflow is untested; the parts most likely to need a
  fix are `rustup toolchain install` with no arguments, bash as the default shell, and the NSIS
  download during bundling.
- Owner review of the limits in `docs/ci.md` (D5), then replace its expected figures with
  measured ones.
- Install the CI artifact on Windows and confirm it opens to the empty state.
- Replace the placeholder icon `src-tauri/app-icon.png` and regenerate with `npx tauri icon`
  before the first distributed build.

### Phase 1: Library and persistence

- Resolve and create the three storage roots. Write and verify `library.json`; refuse to open a
  library whose ID does not match the database, with an actionable error.
- SQLite with WAL and foreign keys. Forward-only migrations: before applying, take a database
  snapshot, verify it opens and passes an integrity check, apply inside a transaction, and
  restore the snapshot on failure. Record an in-progress marker so an interrupted migration is
  detected and recovered on the next start.
- Database access on a dedicated worker, never on the UI thread; short transactions only.
- Corruption handling: integrity check on open; on failure, stop, explain, and offer restore
  from a named snapshot. Never delete or overwrite the damaged file.
- Typed error model and the command pattern from section 3.6.
- Exit: migration success, failure-rollback and interruption-recovery tests pass; opening a
  corrupt database yields the recovery screen and leaves the file untouched.

Status: implemented; exit criteria met in WSL. Gaps handed to Phase 10:

- A missing or unreadable `library.json` has no in-app repair; the user must put the file back.
- The branch that restores the snapshot after a failed migration is exercised only through the
  interrupted-migration tests, which share its code.
- Snapshots exist only before migrations, so a library that has never been migrated has
  nothing to restore from.

### Phase 2: Prepare

- Meeting list with real empty, loading and error states. Create a meeting; edit title and
  optional planned time.
- Agenda items: add, edit, remove, reorder, by keyboard as well as pointer. Reordering is a
  single transactional command. Agenda edits are rejected once recording has started.
- Delete moves a meeting to trash after confirmation; restore from trash; permanent deletion is
  a second, separately confirmed action.
- Settings storage and the section shortcut setting (stored only; used in Phase 4).
- Implement section 3.5 in `skribe-domain` with unit and property tests, ahead of its first use.
- Exit: a meeting with an agenda survives restart; revision conflicts are reported, not
  overwritten; the timeline rule tests pass.

Take note (section 3.8):

- Start only after the first green CI run, so a CI failure is not mistaken for a Phase 2 defect.
- This phase adds the first real tables, as migration 2. On any machine with an existing
  library it is the first migration of real data and takes the first snapshot.
- Add `MeetingId` and the other IDs to `skribe-domain` in the form of `LibraryId`: canonical
  version 7 only. IDs cross the command boundary as strings and are parsed in `skribe-ipc`.
- A revision conflict is a new `StoreError` variant and `ErrorCode`, and its text must say what
  was kept.
- Trash and permanent-delete confirmations follow the inline confirmation pattern. Extract it
  and the button into `src/ui/` here, since this is their second use.
- Replace the placeholder text in `src/features/meetings/MeetingsHome.tsx`; it currently says
  no meeting features exist.
- Planned time is stored as Unix milliseconds unless a date-time crate is added deliberately.
- Record the timeline rules in `docs/timeline-rules.md` once D6 is confirmed.

### Phase 3: Durable two-track recording

Spike first (Windows machine required): capture microphone and loopback together for at least
two hours, confirm timestamps, silence behaviour and drift, and settle the file format.

- Capture trait with a WASAPI backend: shared-mode microphone capture and render loopback
  capture, each on its own thread, each in the device's native format.
- Common clock: one monotonic origin per recording. Every packet's device timestamp is recorded
  in `<track>.clock`, so sample position maps to timeline time without assuming the nominal
  sample rate. This is what handles drift between the two devices.
- Loopback delivers no packets while nothing is playing. Represent that as timeline gaps in the
  clock map, not as missing time, so both tracks stay aligned.
- Bounded pipeline: capture thread → fixed-size ring buffer → writer thread. On overflow, count
  the dropped frames, record a health event with the gap, and keep capturing.
- Writer: append PCM to a WAV container that can exceed 4 GB (RF64), flush on a short fixed
  interval, and keep a journal sufficient to repair the header after a crash.
- Preflight before start: both devices present, formats readable, enough free disk for a stated
  minimum duration, library writable. Start is refused with a specific reason otherwise.
- Coordinated start and stop across both sources. Finalise each track: flush, write final
  header, compute SHA-256, record frame count, offset and health. `recorded` is reached only
  when both finalise; otherwise report exactly which track failed and why.
- Live per-track health in the UI: level, dropped frames, device state. Device loss follows D13.
- Recovery: rebuild an interrupted track from its journal into a valid file, mark the meeting
  `interrupted`, and show what was recovered.
- Exit: recordings of two hours stay within a fixed memory bound; killing the process
  mid-recording loses no more than the flush interval; unplugging a device preserves the other
  track; nothing in this phase requires a model or network.

Take note (section 3.8):

- WASAPI needs `unsafe`. Override the workspace `unsafe_code` lint in `skribe-audio` only, and
  keep the unsafe code in the `cfg(windows)` backend behind the capture trait.
- WSL does not compile the `cfg(windows)` backend, so clippy and tests there say nothing about
  it. CI is the only compile check: keep that layer thin and push it in small steps.
- Stop and finalisation must be driven explicitly, including when the window is closed during
  a recording. The process does not run destructors on exit.
- Live track health is the first use of typed events: add the event permission and payload
  types as section 3.8 describes.
- Once a recording exists, `audio/` is no longer empty, so a missing database stops the app
  instead of creating a new one. That is intended; recovery tests should expect it.
- The free-disk preflight is new code; nothing in Phase 1 measures free space.

### Phase 4: Live notes and sections

- Recording session service in Rust owns live time. Every note and transition is stamped in
  Rust on receipt and written immediately.
- Notes: the frontend sends text with a retry key. A retry with the same key returns the
  existing note. Editing text adds a revision and never changes the timestamp or key.
- Section shortcut: registered globally from the configured setting so it works while another
  application has focus. Registration failure (for example a conflict) is reported with a way
  to choose another shortcut. An on-screen control performs the same action.
- Transitions follow rules 8 and 9 in section 3.5.
- Record screen: elapsed time from Rust, current section, upcoming agenda, note entry that keeps
  focus, both track health indicators, stop.
- Exit: notes and transitions survive a crash mid-recording; duplicate submissions create one
  note; triggering past the end of the agenda creates additional sections.

Take note (section 3.8):

- Register the global shortcut from Rust, never from the frontend, so the window needs no
  shortcut permission and the capability test keeps its meaning. The plugin is wired in
  `src-tauri`; the advance and exhaustion logic stays in the library crates.
- Each note and transition is its own short transaction on the database worker. Commits are
  already durable through power loss (`synchronous = FULL`).
- The retry key for notes is enforced by a unique constraint in the schema, not by the frontend.

### Phase 5: Review without models

- Review screen driven by the effective timeline: sections, notes, section navigation.
- Section edits: rename, insert boundary, move boundary, remove boundary, each stored as an
  override. Original events are never modified.
- Playback through the audio URI scheme, with a derived playback mix built on demand into
  `derived/`. Clicking a note or section seeks playback; playback position highlights the
  current section. Per-source playback where it helps.
- Local search over notes and section titles.
- Unsaved-draft guard on navigation and window close.
- Exit (milestone M1): a user with no models installed can prepare, record, annotate, review,
  correct boundaries and play back, entirely offline.

Take note (section 3.8):

- The audio URI scheme is a new origin: add it to `media-src` in the Content Security Policy
  and check playback in a built app, on Windows as well as Linux, because the scheme's URL
  form differs between them.
- Register the scheme in `src-tauri`, but keep ID validation, path resolution and range parsing
  in a library crate so they are tested.
- The window-close guard for unsaved drafts needs either a Rust-side close handler or a window
  permission. Prefer the Rust-side handler; a window permission changes the capability test.
- Search can use FTS5; the bundled SQLite already includes it.

### Phase 6: Model and runtime management

Spike first: run each candidate runtime (D9) as a child process on a CPU-only Windows machine
against sample audio and text; measure speed, memory and package size; confirm licences permit
redistribution or direct download.

- Catalogue schema and reviewed catalogue: per package its kind, capabilities, languages,
  version, licence, provenance, download URL, size, SHA-256, expected file structure, hardware
  and memory requirements, and the exact runtime versions it works with. UI reads the catalogue;
  it holds no model-specific logic.
- Catalogue refresh is user-initiated and signature-checked (D11). The embedded catalogue keeps
  the app usable offline.
- Download: user-initiated only; streamed to `staging/`; cancel, resume by byte range, retry;
  disk space checked first.
- Verify size and SHA-256. Extract safely: reject absolute paths, parent traversal, links,
  entries outside the expected structure, and anything exceeding the declared size. Never run a
  script or installer from a package.
- Validate structure against the catalogue, then run a runtime check (start the runtime, load
  the model, run a tiny fixed input). Only then move the package into `packages/` with an
  atomic rename and register it in one transaction.
- Updates install beside the working version and switch selection only after validation.
  A failed update leaves the previous version selected and usable.
- Installing a model offers to install the runtime it needs, through the same checks.
- Compatibility check per machine: CPU features, memory, disk, GPU where relevant. Report
  "not supported on this machine" plainly.
- Removal is confirmed, and refused while a package is in use by a job or required by an
  installed model.
- Exit: interrupted, corrupted and malicious-archive downloads never produce a ready package;
  with the network disabled, installed packages still validate and load.

Take note (section 3.8):

- Downloads happen in Rust. The frontend's `connect-src` stays limited to Tauri's IPC and gains
  no remote origin.
- This phase adds the first HTTP, TLS and archive dependencies. Check the installer against the
  30 MiB budget in `docs/ci.md` after adding them, and raise the budget only as a reviewed change.
- `<models>` exists but is empty; create `staging/` and `packages/` here. This is also the
  first phase with leftovers to clean up, so decide what is cleared at startup, in `staging/`
  and in `<temp>`.
- A package that fails validation is reported through the same typed error model, including
  what was kept and whether the previous version is still selected.

### Phase 7: Processing pipeline

- Job runner: one job at a time, off the UI thread, stages persisted in `job_stages` with
  dependencies, progress computed from completed units, cancellation points between units, and
  retry from the last checkpoint. No database transaction is held during inference.
- Meetings that are `recorded` without a usable model set are `waiting_for_models` and are
  queued when setup completes, on user confirmation.
- Stages:
  1. Audio preparation: per track, resample to the model's input format using the clock map.
     Originals are read-only.
  2. Diarisation: per track. Speakers are per-track model clusters with no identity attached.
  3. Turn planning: convert speaker regions into bounded transcription units.
  4. Transcription: per unit, checkpointed per unit, storing original text and timestamps.
  5. Reconciliation: merge both tracks onto the timeline in chronological order; flag
     cross-track overlap and probable echo (the same speech on both tracks) for review rather
     than removing it; write segments in one short transaction per batch.
- Each run is stored under a new `processing_runs` row with exact package versions.
- Exit: cancel and resume at every stage produce the same result as an uninterrupted run;
  a failed stage leaves the meeting, audio and notes intact with an explanation and a retry.

Take note (section 3.8):

- The database worker is serial. Segment batches must be small enough that a note edit or a
  status query made during processing is not kept waiting.
- Marking interrupted jobs at startup belongs in the library open path, beside the
  interrupted-migration recovery, and is reported to the user the same way (an open notice).
- The stub runtime for tests is its own test-only target outside `src-tauri`. `tauri build`
  compiles only the app package, which keeps the stub out of the installer.
- Job progress uses the typed events introduced in Phase 3.

### Phase 8: Transcript review

- Chronological transcript interleaved with notes, grouped by section, with speaker labels,
  timestamps and source track.
- Edit segment text and speaker label as overrides; the original output stays viewable.
  Speaker labels are user text attached to a cluster, shown as distinct from the model ID.
  Flags from reconciliation are visible and dismissible.
- Full-text search across effective transcript text and notes, with jump-to-result.
- Aligned playback: seek from any segment; highlight the segment being played.
- Boundary edits recompute membership and staleness without touching transcription.
- Reprocessing per D14.
- Long transcripts are virtualised so a multi-hour meeting stays responsive.
- Exit (milestone M2): record → process → review → correct works offline on the reference
  machine, and edits survive navigation, restart and reprocessing.

Take note (section 3.8):

- Choose the virtualisation approach with the Content Security Policy in mind: a library that
  injects `<style>` elements will not work. Check the choice in a built app before building on it.
- Full-text search uses FTS5 from the bundled SQLite. Index rebuilds after an edit are short
  transactions on the worker, not one long one.

### Phase 9: Minutes

- Generation is a separate, explicit user action, run as a job with per-section checkpoints.
- Input per section: effective title, member segments and notes with their IDs. Additional
  sections and unvisited agenda items are included; a section with no evidence is reported as
  having none rather than being filled in.
- Output is structured: items with a kind (summary point, decision, action), text, the IDs of
  supporting segments and notes, and an evidence marker (sufficient, ambiguous, insufficient).
  Items citing IDs that do not exist in the input are rejected. Owners and deadlines appear only
  when present in cited sources, and are always shown as needing review.
- Sections longer than the model's context are processed in bounded parts that keep their
  source links.
- Revisions: generated and manual revisions are stored; manual edits create a new revision.
- Staleness: each revision stores a fingerprint of its inputs (member IDs and their effective
  text). A changed fingerprint marks that section stale. A rename does not change it.
- Regeneration produces a candidate. The user compares it with the current revision and adopts
  it explicitly, per section. Nothing is replaced automatically.
- Source links in the minutes navigate to the segment or note and its audio.
- Exit (milestone M3): the full workflow in AGENTS.md runs offline end to end.

Take note (section 3.8):

- Minutes generation reuses the Phase 7 job runner and events. Rejected output (items citing
  IDs that are not in the input) is a typed error like any other, with what was kept.
- Prompts and model output never go into `CommandError.detail`, which is for paths and storage
  error text only.

### Phase 10: Hardening and release preparation

- Backup: user-initiated database snapshot (clearly labelled as excluding audio and models) and
  full library backup, which requires the library to be closed. Restore with verification.
- Fault injection across recording, migration, download, install and each pipeline stage.
- Accessibility pass: full keyboard operation, focus management, names and roles for all
  controls, live-region announcements for state changes, contrast, reduced motion.
- Installer: confirm update preserves the library and models; confirm default uninstall
  preserves both and deletion is an explicit choice.
- Performance on the reference low-end machine; document supported hardware and languages
  without overstating them.
- Windows acceptance checklist, code signing and publication are release steps outside CI.

Take note (section 3.8 and the Phase 0 and 1 status notes):

- User-initiated snapshots must use the `skribe-v<schema>-<time>.sqlite` name form, or the
  restore code must be extended to accept another. A full backup takes the library lock, which
  is what "closed" means in the code.
- Decide retention for `backups/` and `db/set-aside-*`. Nothing removes them today; removal
  must be explicit and confirmed.
- Add an in-app repair for a missing or unreadable `library.json`.
- Fault injection covers the snapshot restore after a failed migration, which has no direct test.
- Verify on Windows what `docs/storage.md` only expects: update keeps the library and models,
  and the uninstaller deletes app data only when the user ticks that option.
- Decide the WebView2 install mode. The default bootstrapper needs a connection on a machine
  without WebView2; the offline installer adds over 100 MB.
- Add `eslint-plugin-jsx-a11y` to the accessibility pass once it supports the ESLint in use,
  and move to TypeScript 7 when `typescript-eslint` supports it.
- Decide whether a second launch should focus the running window instead of showing "library
  in use".

## 5. Milestones

| Milestone | After phase | What a user can do |
|-----------|-------------|--------------------|
| M0 | 1 | Install and open the app; data layer is safe |
| M1 | 5 | Prepare, record both tracks, take notes, mark sections, review and play back, with no models |
| M2 | 8 | Install models, process recordings locally, review and correct the transcript |
| M3 | 9 | Generate, edit and regenerate evidence-linked minutes |
| M4 | 10 | Release candidate ready for signing and acceptance |

## 6. CI and installer builds

One workflow, one job on a hosted Windows x64 runner, steps strictly in sequence so that the
installer is built from the revision that just passed:

1. Checkout; install toolchains pinned by `rust-toolchain.toml` and the Node version file.
2. `npm ci`; lint; type-check; frontend tests.
3. Build the frontend bundle. The app crate embeds it at compile time, so clippy needs it.
4. `cargo fmt --check`; `cargo clippy` with warnings denied; `cargo test --workspace`. The tests
   also fail when `src/ipc/generated` is out of date.
5. `tauri build` for the NSIS target. This is the only step that performs the release compile.
6. `scripts/ci/package-manifests.mjs` generates SHA-256 checksums, a source manifest (commit,
   lockfile hashes, toolchain versions) and a build manifest (runner image, limits, artifact
   sizes), and enforces the installer and disk budgets. It refuses a working tree that differs
   from the checked-out commit.
7. Upload installer, checksums and manifests as workflow artifacts.

Any failed step stops the job, so packaging cannot run on a failing revision. Build parallelism,
timeout and disk budget follow `docs/ci.md`. Artifacts are unsigned build outputs, not releases.
Models and runtimes are never bundled; a CI check fails if the installer exceeds a size budget.
The workflow exists as `.github/workflows/ci.yml` and has not yet run (Phase 0 status).

## 7. Testing

| Layer | What | Where it runs |
|-------|------|---------------|
| Domain | Timeline derivation, tie and boundary rules, state machines, staleness; property tests | WSL and CI |
| Store | Migrations, rollback, interruption recovery, revision conflicts, FTS | WSL and CI |
| Command contract | Error mapping, command handlers, generated TypeScript in sync with Rust, capability file grants exactly the app's commands | WSL and CI |
| Audio | Writers, journal recovery, clock map, overflow, using the synthetic backend | WSL and CI |
| Audio on devices | WASAPI capture, loopback silence, device loss, long recordings | Windows machine, manual checklist |
| Models | Resume, hash mismatch, hostile archives, atomic promotion, against a local test server | WSL and CI |
| Pipeline | Stage ordering, cancel and resume, checkpoint equivalence, using a stub runtime executable built for tests | WSL and CI |
| Inference | Real runtimes and models on sample audio | Windows machine, not hosted CI |
| Frontend | Controllers and components, keyboard paths, error states | WSL and CI |
| End to end | Installed app, full workflow, offline | Windows machine, before release |

Stub runtimes and synthetic capture are test fixtures. They are never compiled into a release
build and never produce output shown to a user as a meeting result.

The Tauri app crate itself has no automated tests. What it adds over the library crates (the
webview, the Content Security Policy, the capability file as enforced at runtime) is checked by
running a built app.

## 8. Risks

| Risk | Effect | Mitigation |
|------|--------|------------|
| Loopback capture stalls during silence, and the two devices drift | Tracks misalign over long meetings | Per-packet clock map; Phase 3 spike with a two-hour recording before building on it |
| Capture devices change mid-meeting | Lost or mislabelled audio | D13; per-track health; never substitute a source |
| Speakerphone use puts remote speech on the microphone track | Duplicate text, wrong attribution | Flag probable echo in reconciliation; require human review; never auto-delete |
| CPU-only inference is slow on modest hardware | Processing takes longer than the meeting | Measure in the Phase 6 spike; small default models; real progress and safe cancel; state hardware limits honestly |
| Small local language models produce unsupported claims | Minutes that look authoritative but are wrong | Structured output with mandatory source IDs, validation, evidence markers, human adoption |
| Model and runtime licences restrict redistribution | Cannot ship a default set | Licence review per catalogue entry in Phase 6 before anything depends on it |
| Hosted-runner time, memory or disk limits | CI cannot build the installer | Single compile job, bounded parallelism, limits documented in `docs/ci.md`, no models in CI |
| Global shortcut conflicts with other applications | Section transitions silently fail | Report registration failure; on-screen control as an equal path |
| WSL cannot exercise audio or the installer | Defects found late | Windows-only checklists per phase; CI on Windows from Phase 0 |
| The CI workflow and its limits have never run | The first push fails for reasons unrelated to the change being pushed | Get the first green run before starting Phase 2; then record measured figures in `docs/ci.md` |
| The Content Security Policy and capability list reject a dependency or origin only at runtime | Can work under `tauri dev` and fail in the installed app | Check every new UI dependency and origin in a built app (section 3.8) |
| Bundle identifier or library location changed after release | Users appear to lose data | Fix D1 and D7 before the first distributed build; changes only via explicit verified migration |

## 9. Requirement coverage

| AGENTS.md section | Phases |
|-------------------|--------|
| Reliable recording and lifecycle | 3, 4, 7 (gating of inference), 10 |
| Authoritative timeline and editable review | 2, 4, 5, 8 |
| Local processing and evidence-based minutes | 7, 9 |
| Model distribution and compatibility | 6 |
| Persistence, privacy and native security | 0 (capabilities), 1, 3.6 and 3.7 throughout, 6, 10 |
| Accessible and truthful interface | Every phase; audited in 10 |
| Development workflow, CI/CD and installer builds | 0, maintained throughout |

## 10. Not planned

- Pause and resume. If added later, gaps must be preserved on the timeline.
- Any cloud inference, telemetry or upload of meeting content.
- Identifying real people from voice, or assuming the microphone track is the local user.
- Non-Windows delivery targets.
- Code signing, acceptance sign-off and publication, which are release steps separate from this plan's CI.
