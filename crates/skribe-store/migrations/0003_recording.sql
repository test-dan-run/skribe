-- When the recording started, by the wall clock for display, and how long it ran on its own
-- monotonic clock.
ALTER TABLE meetings ADD COLUMN recording_started_at_unix_ms INTEGER;
ALTER TABLE meetings ADD COLUMN recording_duration_us INTEGER;

-- One original audio track of a recording. A recording has exactly two: the microphone and
-- the system loopback. Times are microseconds from the recording's origin.
CREATE TABLE tracks (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL REFERENCES meetings (id) ON DELETE CASCADE,
    source TEXT NOT NULL CHECK (source IN ('microphone', 'system')),
    device_id TEXT NOT NULL,
    device_name TEXT NOT NULL,
    -- The device's native format, in which the track is stored unchanged.
    sample_rate INTEGER NOT NULL,
    channels INTEGER NOT NULL,
    bits_per_sample INTEGER NOT NULL,
    valid_bits INTEGER NOT NULL,
    sample_kind TEXT NOT NULL CHECK (sample_kind IN ('int', 'float')),
    channel_mask INTEGER NOT NULL,
    -- Relative to the library folder, with forward slashes.
    audio_path TEXT NOT NULL,
    finalisation TEXT NOT NULL DEFAULT 'pending' CHECK (finalisation IN (
        'pending', 'finalised', 'recovered', 'failed'
    )),
    health TEXT NOT NULL DEFAULT 'healthy' CHECK (health IN ('healthy', 'degraded', 'failed')),
    -- Why the track is degraded or failed: a stable code and technical detail.
    problem TEXT,
    problem_detail TEXT,
    -- Where on the timeline the track stopped, when it stopped before the recording did.
    stopped_at_us INTEGER,
    -- Timeline time of the first frame and just after the last. Null for a track that holds
    -- no audio, such as system audio when nothing was played.
    start_us INTEGER,
    end_us INTEGER,
    frame_count INTEGER,
    data_bytes INTEGER,
    dropped_frames INTEGER NOT NULL DEFAULT 0,
    -- SHA-256 of the finalised file, in lower-case hexadecimal.
    sha256 TEXT,
    UNIQUE (meeting_id, source)
) STRICT;

-- What happened to a track while it was recorded, in the order it was saved.
CREATE TABLE track_health_events (
    id INTEGER PRIMARY KEY,
    track_id TEXT NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    at_us INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN (
        'overflow', 'device_lost', 'capture_failed', 'write_failed', 'events_discarded'
    )),
    -- For an overflow: the frames dropped and where the gap ends.
    frames INTEGER,
    until_us INTEGER,
    detail TEXT
) STRICT;

CREATE INDEX track_health_events_by_track ON track_health_events (track_id, id);
