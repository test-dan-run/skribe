# Skribe: development instructions

## Project purpose

Skribe is an offline-first Windows meeting application that connects recorded conversations
and timestamped notes to searchable transcripts and editable, section-based minutes. It uses
Tauri 2, Rust, React, TypeScript and Vite. Retain this stack, its lockfiles and module boundaries.
Windows is the delivery target; WSL is a development environment.

This document defines the product direction, requirements and development rules.

## Problem statement

Meeting participants need to follow the discussion while capturing notes, topic changes,
decisions and actions. Notes can miss context, and recordings take time to search and turn
into useful minutes. The discussion may depart from the planned agenda, so the meeting record
must reflect the topics actually covered and their place on the timeline.

Users need to review this record without sending meeting content to external services.
Recording and note-taking must remain available without AI models; processing failures must
preserve the saved meeting. Once compatible models and runtimes are installed, core workflows
must work without an Internet connection. Generated text needs traceable evidence and human
review rather than assumptions about accuracy or speaker identity.

## Proposed solution

Connect preparation, durable recording, local processing and human review in one workflow:

1. **Prepare:** set a title and optional planned time, then add, edit, remove and reorder agenda items.
2. **Record:** capture microphone and system audio separately while saving timestamped notes
   and section transitions. A configurable shortcut advances through the agenda and creates
   additional sections when the discussion goes beyond it.
3. **Process:** stop and verify finalisation of both tracks before queuing local processing.
   Saved recordings wait when model setup is incomplete. New runs prepare audio, diarise,
   plan speaker turns, transcribe and reconcile results with the meeting timeline.
4. **Review:** search the chronological transcript alongside notes and sections, use aligned
   playback, and correct text, speaker labels and boundaries while retaining original evidence.
5. **Generate minutes:** use a separate user action to create section-based minutes with source
   links. Save manual revisions, identify stale content and explicitly adopt regenerated candidates.

Support this workflow with in-app model and runtime management. Distribute these packages
separately to keep the application installer small, and require user-initiated setup or updates.
The installed application must operate without end-user developer tools.

## Design considerations and requirements

The following requirements turn the workflow into a durable, reviewable local record.
Preserve them when changing behaviour; surface conflicts and material trade-offs explicitly.

### Reliable recording and lifecycle

- Capture microphone and Windows system loopback as distinct original tracks on a common
  monotonic timeline. Preserve source identity, native formats, offsets and integrity metadata.
  A derived playback mix is acceptable; never replace the originals with an irreversible mix.
- Write incrementally to durable storage with bounded memory. Validate devices and settings
  before starting, coordinate capture and stop across both sources, and finalise both tracks
  before allowing inference.
- Keep recording, processing and minutes states distinct. Use the conceptual flow
  setup → ready → recording → finalising → recorded → processing → review → minutes generation
  → completed, with explicit, recoverable interrupted, failed and cancelled states.
- Models and catalogue access are not prerequisites for recording or timestamped notes.
  Transcription and diarisation run only after capture has stopped and both tracks have
  finalised successfully.
- Report each track's health; never substitute one source for the other or label incomplete
  finalisation as success. Preserve healthy audio, notes and events on failure.
- Pause is currently unavailable. Any future pause and resume support must preserve timeline gaps.

### Authoritative timeline and editable review

- Use stable IDs and explicit relationships for meetings, tracks, agenda items, sections,
  events, notes, transcript segments, speakers, jobs, minutes and installed models.
- Allow agenda items to be added, edited, removed and reordered before recording. Preserve
  historical recording events; save review edits as persistent overrides rather than rewriting history.
- The configurable section shortcut advances to the next agenda item. After exhaustion, each
  trigger creates a persistent additional section with a usable default title; never wrap or ignore it.
- Backend timestamps and persisted event ordering are authoritative. Save notes and transitions
  incrementally; note text edits retain their original timestamps and retry identity.
- Derive section membership from the effective timeline. Renames retain boundary timestamps;
  inserting, moving or removing boundaries updates transcript and note membership and minutes
  staleness together. Preserve established rules for tied events, zero and end boundaries,
  and the initial boundary.
- Support chronological transcript and notes, speaker labels, timestamps, sections, section
  navigation and edits, local search and aligned playback. Expose source attribution where useful.
- Transcript edits retain original output, audio and timestamps. User speaker labels are distinct
  from model IDs; do not assume microphone means the local user or clusters identify real people.
  Echo, overlap and uncertain attribution require human review.
- Preserve edits across navigation and reopening, and guard unsaved drafts. Reprocessing retains
  previous results and manual edits; boundary changes alone must not rerun transcription.

### Local processing and evidence-based minutes

- Keep audio preparation, diarisation, turn planning, transcription, reconciliation and minutes
  generation as explicit stages with dependencies, real progress, safe cancellation and checkpoint
  retries. Use bounded processing units and durable checkpoints; run expensive work off the UI
  thread and outside long-held database locks.
- Minutes use the current section structure, relevant transcript and notes, including additional
  and unvisited agenda sections. Mark insufficient or ambiguous evidence; never invent confidence,
  words, identities, decisions, owners or deadlines. Persist source links and editable revisions.
- Mark affected minutes stale when source content or boundaries change. Renaming alone need not
  make content stale. Regeneration preserves user edits through explicit comparison and adoption.

### Model distribution and compatibility

- Model capabilities, languages, versions, licences, provenance, sizes, hardware requirements
  and exact runtime compatibility belong in maintained catalogue metadata, not scattered UI logic.
- Models and runtimes stay outside application binaries and meeting storage. Keep the installer small.
  Download only during user-initiated setup or updates; installed compatible models and all core
  meeting workflows must remain usable offline. No end-user Python, Node, Rust, SDK or WSL requirement.
- Stream downloads into staging; support cancellation, resume and retry, trusted integrity checks,
  safe extraction, structure and runtime validation, and atomic promotion and registration. Never mark
  partial packages ready or replace a working version before its replacement is validated.
- Validate hardware, memory, disk and runtime availability; offer CPU operation where feasible
  without implying support for every Windows machine or language. Protect in-use packages and
  runtime dependencies on removal, and preserve selections and usable versions through failures.
  Automatic dependency installation must retain these checks and failure protections.

### Persistence, privacy and native security

- Rust owns IDs, live time, lifecycle, SQLite transactions and migrations, and privileged operations.
  Use narrow typed Tauri commands and capabilities; validate IDs, revisions, content, paths and limits.
  Never expose arbitrary frontend filesystem access, URLs, shell execution or runtime arguments.
- Keep audio, models, temporary files and structured data separate.
  Do not silently change storage identity or migrate destructively.
- Maintain library ownership, verified migration backups, rollback and interruption recovery.
  Database snapshots exclude audio and models; a full backup requires the complete closed library.
  Corruption must produce actionable errors without silently deleting or overwriting recoverable data.
- Meeting audio, transcripts, notes and minutes stay local. No cloud inference or fallback, telemetry
  or diagnostic uploads of meeting content; transmission requires separate explicit user approval.
  Treat local runtime output, prompts, checkpoints and backups as sensitive content, not logs.
- Model metadata and packages are untrusted until validated against reviewed provenance and
  integrity metadata. Reject traversal, unsafe archives or links, and arbitrary package scripts
  or installers.
- Deletion and removal must be explicit and confirmed. Trash is recoverable; application updates and
  default uninstall must preserve meeting data and installed models unless the user chooses deletion.

### Accessible and truthful interface

- Provide keyboard access, accessible controls and truthful loading, empty, error and success
  states. Explain failures, retained data and recovery actions; polling must not erase actionable errors.
- Present real behaviour and progress. Visual placeholders and fabricated AI output must never
  be presented as functioning features or actual meeting results.

## Development workflow

### Scope and implementation

- Inspect relevant source, applicable instructions, design and tests before changing behaviour.
  Prefer focused incremental changes; avoid unrelated rewrites, dependencies and speculative abstractions.
  Keep presentation, controllers, native services, inference and persistence separate and typed.
- Respect the user's current scope and preserve existing work. Do not silently weaken requirements
  or describe planned work as implemented.

### GitHub Actions CI/CD and installer builds

- Use GitHub Actions CI/CD to produce the Windows x64 NSIS installer. Keep automated checks and
  installer packaging connected: package the same revision only after all required checks pass.
- Retain sequential checks, one compiler/CMake job and the documented hosted-runner resource limits.
- Upload the installer, checksums and source/build manifests as workflow artifacts so each
  build is traceable and reviewable. Successful CI packaging does not establish release
  readiness; signing, Windows acceptance and publication remain separate release steps.