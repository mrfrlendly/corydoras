use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;
use std::time::Duration;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS languages (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS items (
    id              TEXT PRIMARY KEY,
    language_id     TEXT NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL DEFAULT 'image',
    file_name       TEXT NOT NULL,
    original_name   TEXT NOT NULL DEFAULT '',
    source          TEXT,
    width           INTEGER NOT NULL DEFAULT 0,
    height          INTEGER NOT NULL DEFAULT 0,
    hash            TEXT NOT NULL DEFAULT '',
    added_at        TEXT NOT NULL,
    tags            TEXT NOT NULL DEFAULT '[]',
    note            TEXT NOT NULL DEFAULT '',
    weight          INTEGER NOT NULL DEFAULT 3,
    anti            INTEGER NOT NULL DEFAULT 0,
    analysis_status TEXT NOT NULL DEFAULT 'pending',
    analysis_error  TEXT
);
CREATE INDEX IF NOT EXISTS idx_items_language ON items(language_id);
CREATE INDEX IF NOT EXISTS idx_items_hash ON items(language_id, hash);

CREATE TABLE IF NOT EXISTS regions (
    id      TEXT PRIMARY KEY,
    item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    ord     INTEGER NOT NULL DEFAULT 0,
    x       REAL NOT NULL,
    y       REAL NOT NULL,
    w       REAL NOT NULL,
    h       REAL NOT NULL,
    note    TEXT NOT NULL DEFAULT '',
    tag     TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_regions_item ON regions(item_id);

CREATE TABLE IF NOT EXISTS analyses (
    item_id    TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
    json       TEXT NOT NULL,
    model      TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS versions (
    language_id TEXT NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,
    created_at  TEXT NOT NULL,
    model       TEXT NOT NULL,
    item_ids    TEXT NOT NULL DEFAULT '[]',
    edited      INTEGER NOT NULL DEFAULT 0,
    input_hash  TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (language_id, version)
);

CREATE TABLE IF NOT EXISTS projects (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pins (
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    language_id TEXT NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,
    role        TEXT NOT NULL,
    scope       TEXT NOT NULL DEFAULT 'all',
    PRIMARY KEY (project_id, language_id)
);

CREATE TABLE IF NOT EXISTS jobs (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    kind       TEXT NOT NULL,
    target     TEXT NOT NULL,
    run_after  INTEGER NOT NULL,
    attempts   INTEGER NOT NULL DEFAULT 0,
    gen        INTEGER NOT NULL DEFAULT 1,
    last_error TEXT,
    created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_jobs_kind_target ON jobs(kind, target);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS usage (
    month         TEXT PRIMARY KEY,
    input_tokens  INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    requests      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS sources (
    id              TEXT PRIMARY KEY,
    language_id     TEXT NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,
    title           TEXT NOT NULL DEFAULT '',
    url             TEXT,
    site            TEXT NOT NULL DEFAULT '',
    description     TEXT NOT NULL DEFAULT '',
    media           TEXT NOT NULL DEFAULT 'page',
    file_name       TEXT,
    original_name   TEXT NOT NULL DEFAULT '',
    has_preview     INTEGER NOT NULL DEFAULT 0,
    text_chars      INTEGER NOT NULL DEFAULT 0,
    word_count      INTEGER NOT NULL DEFAULT 0,
    hash            TEXT NOT NULL DEFAULT '',
    added_at        TEXT NOT NULL,
    note            TEXT NOT NULL DEFAULT '',
    weight          INTEGER NOT NULL DEFAULT 3,
    analysis_status TEXT NOT NULL DEFAULT 'pending',
    analysis_error  TEXT,
    analysis        TEXT,
    analysis_model  TEXT
);
CREATE INDEX IF NOT EXISTS idx_sources_language ON sources(language_id);

CREATE TABLE IF NOT EXISTS rule_sets (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
    description TEXT NOT NULL DEFAULT '',
    always      INTEGER NOT NULL DEFAULT 0,
    rules       TEXT NOT NULL DEFAULT '[]',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS project_rule_sets (
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    rule_set_id TEXT NOT NULL REFERENCES rule_sets(id) ON DELETE CASCADE,
    PRIMARY KEY (project_id, rule_set_id)
);

-- Version 0.2 process profiles. Version 0.3 moves them into rule_sets.
CREATE TABLE IF NOT EXISTS processes (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
    description TEXT NOT NULL DEFAULT '',
    constraints TEXT NOT NULL DEFAULT '[]',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS proposals (
    id             TEXT PRIMARY KEY,
    language_id    TEXT NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    base_version   INTEGER NOT NULL,
    summary        TEXT NOT NULL,
    rationale      TEXT NOT NULL DEFAULT '',
    author         TEXT NOT NULL DEFAULT '',
    patch          TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'pending',
    created_at     TEXT NOT NULL,
    decided_at     TEXT,
    result_version INTEGER
);
CREATE INDEX IF NOT EXISTS idx_proposals_language ON proposals(language_id, status);

CREATE TABLE IF NOT EXISTS outputs (
    id               TEXT PRIMARY KEY,
    language_id      TEXT NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    project          TEXT NOT NULL DEFAULT '',
    name             TEXT NOT NULL,
    file_name        TEXT NOT NULL,
    width            INTEGER NOT NULL DEFAULT 0,
    height           INTEGER NOT NULL DEFAULT 0,
    hash             TEXT NOT NULL DEFAULT '',
    language_version INTEGER,
    author           TEXT NOT NULL DEFAULT '',
    note             TEXT NOT NULL DEFAULT '',
    family           TEXT NOT NULL DEFAULT '',
    measurements     TEXT NOT NULL DEFAULT '[]',
    check_json       TEXT,
    verdict          TEXT NOT NULL DEFAULT 'pending',
    verdict_note     TEXT NOT NULL DEFAULT '',
    item_id          TEXT REFERENCES items(id) ON DELETE SET NULL,
    created_at       TEXT NOT NULL,
    decided_at       TEXT
);
CREATE INDEX IF NOT EXISTS idx_outputs_language ON outputs(language_id, verdict);
"#;

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut st = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = st
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names.iter().any(|n| n == column))
}

/// Adds the columns of later versions to a library made by an earlier version.
/// Each step checks first, so the migration can run on every open.
pub fn migrate(conn: &Connection) -> Result<()> {
    let steps: [(&str, &str, &str); 6] = [
        ("versions", "schema", "ALTER TABLE versions ADD COLUMN schema INTEGER NOT NULL DEFAULT 1"),
        ("versions", "warnings", "ALTER TABLE versions ADD COLUMN warnings TEXT NOT NULL DEFAULT '[]'"),
        ("projects", "process_id", "ALTER TABLE projects ADD COLUMN process_id TEXT"),
        // Canvas view (0.6): place of each item; NULL until placed.
        ("items", "canvas_x", "ALTER TABLE items ADD COLUMN canvas_x REAL"),
        ("items", "canvas_y", "ALTER TABLE items ADD COLUMN canvas_y REAL"),
        ("items", "canvas_w", "ALTER TABLE items ADD COLUMN canvas_w REAL"),
    ];
    for (table, column, sql) in steps {
        if !has_column(conn, table, column)? {
            conn.execute_batch(sql)?;
        }
    }
    Ok(())
}

/// Opens (and creates, if necessary) the library database in WAL mode.
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")?;
    conn.execute_batch(SCHEMA)?;
    migrate(&conn)?;
    Ok(conn)
}
