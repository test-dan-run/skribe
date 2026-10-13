//! Opening and inspecting the SQLite file. Nothing here changes a database that has not first
//! passed [`inspect`].

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension};

use crate::StoreError;

/// How many integrity problems to collect before stopping; enough to describe the damage.
const MAX_INTEGRITY_MESSAGES: u32 = 20;

/// What a database file contains, read without modifying it.
#[derive(Debug)]
pub(crate) struct Inspection {
    pub schema_version: u32,
    /// `library_meta.library_id`, when that table and row exist.
    pub library_id: Option<String>,
    pub has_tables: bool,
}

/// Runs the integrity check and reads the file's identity over a read-only connection, so a
/// damaged or foreign file is reported exactly as it was found. A read-only connection never
/// checkpoints the write-ahead log into the main file.
pub(crate) fn inspect(path: &Path) -> Result<Inspection, StoreError> {
    let fail = |error: rusqlite::Error| classify(path, "inspect", error);

    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(fail)?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(fail)?;

    let problems = integrity_problems(&conn).map_err(fail)?;
    if !problems.is_empty() {
        return Err(StoreError::DatabaseCorrupt {
            path: path.to_owned(),
            detail: problems.join("; "),
        });
    }

    let schema_version = user_version(&conn).map_err(fail)?;
    let table_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .map_err(fail)?;
    let has_meta: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_schema \
             WHERE type = 'table' AND name = 'library_meta')",
            [],
            |row| row.get(0),
        )
        .map_err(fail)?;
    let library_id = if has_meta {
        conn.query_row(
            "SELECT library_id FROM library_meta WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(fail)?
    } else {
        None
    };

    Ok(Inspection {
        schema_version,
        library_id,
        has_tables: table_count > 0,
    })
}

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens the database for use: write-ahead logging, durable commits and enforced foreign keys.
pub(crate) fn open(path: &Path) -> Result<Connection, StoreError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn =
        Connection::open_with_flags(path, flags).map_err(|error| classify(path, "open", error))?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::sqlite("configure"))?;

    let mode: String = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
        .map_err(|error| classify(path, "enable write-ahead logging", error))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::WalUnavailable {
            path: path.to_owned(),
            mode,
        });
    }
    // FULL keeps every committed note and event through power loss, not only a process crash.
    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(StoreError::sqlite("configure"))?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(StoreError::sqlite("configure"))?;
    Ok(conn)
}

pub(crate) fn user_version(conn: &Connection) -> rusqlite::Result<u32> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
}

/// Messages from `PRAGMA integrity_check`; empty when the database is sound.
pub(crate) fn integrity_problems(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement =
        conn.prepare(&format!("PRAGMA integrity_check({MAX_INTEGRITY_MESSAGES})"))?;
    let messages = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(messages
        .into_iter()
        .filter(|message| message != "ok")
        .collect())
}

/// SQLite reports damage either through the integrity check or by refusing to read the file.
fn classify(path: &Path, action: &'static str, error: rusqlite::Error) -> StoreError {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => StoreError::DatabaseCorrupt {
            path: path.to_owned(),
            detail: error.to_string(),
        },
        _ => StoreError::Sqlite {
            action,
            detail: error.to_string(),
        },
    }
}
