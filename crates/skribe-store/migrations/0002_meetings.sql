-- A meeting, from preparation onwards. Recording, processing and minutes are three separate
-- state machines. `revision` guards the title, planned time and agenda against stale edits.
CREATE TABLE meetings (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    planned_start_unix_ms INTEGER,
    recording_state TEXT NOT NULL DEFAULT 'setup' CHECK (recording_state IN (
        'setup', 'ready', 'recording', 'finalising', 'recorded', 'interrupted', 'failed'
    )),
    processing_state TEXT NOT NULL DEFAULT 'none' CHECK (processing_state IN (
        'none', 'waiting_for_models', 'queued', 'running', 'review', 'cancelled', 'failed',
        'interrupted'
    )),
    minutes_state TEXT NOT NULL DEFAULT 'none' CHECK (minutes_state IN (
        'none', 'generating', 'current', 'stale', 'candidate_available', 'cancelled', 'failed'
    )),
    revision INTEGER NOT NULL DEFAULT 1,
    created_at_unix_ms INTEGER NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL,
    -- Set while the meeting is in the trash. Its rows and files stay where they are.
    trashed_at_unix_ms INTEGER
) STRICT;

-- The planned agenda, in `position` order from zero without gaps.
CREATE TABLE agenda_items (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL REFERENCES meetings (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    title TEXT NOT NULL,
    UNIQUE (meeting_id, position)
) STRICT;

-- Application settings, one row per key. A missing row means the default.
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    updated_at_unix_ms INTEGER NOT NULL
) STRICT;
