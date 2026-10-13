-- Identity of the library this database belongs to. Exactly one row.
CREATE TABLE library_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    library_id TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL,
    created_by_app_version TEXT NOT NULL
) STRICT;
