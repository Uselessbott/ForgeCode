use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::proto::Project;

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS projects (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    path       TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS files_index (
    project_id TEXT NOT NULL,
    path       TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    size       INTEGER NOT NULL,
    PRIMARY KEY (project_id, path),
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS event_log (
    seq        INTEGER PRIMARY KEY,
    kind       TEXT NOT NULL,
    payload    BLOB NOT NULL,
    created_at INTEGER NOT NULL
);
";

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.execute_batch(SCHEMA)?;
    Ok(connection)
}

pub fn insert_project(connection: &Connection, project: &Project) -> Result<()> {
    connection.execute(
        "INSERT INTO projects (id, name, path, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![project.id, project.name, project.path, project.created_at],
    )?;
    Ok(())
}

pub fn list_projects(connection: &Connection) -> Result<Vec<Project>> {
    let mut statement = connection
        .prepare("SELECT id, name, path, created_at FROM projects ORDER BY created_at, id")?;
    let rows = statement.query_map([], |row| {
        Ok(Project {
            id: row.get(0)?,
            name: row.get(1)?,
            path: row.get(2)?,
            created_at: row.get(3)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub fn count_projects(connection: &Connection) -> Result<u32> {
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))?;
    Ok(count as u32)
}

pub fn project_exists(connection: &Connection, id: &str) -> Result<bool> {
    let found: Option<i64> = connection
        .query_row("SELECT 1 FROM projects WHERE id = ?1", params![id], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(found.is_some())
}

pub fn upsert_file_index(
    connection: &Connection,
    project_id: &str,
    path: &str,
    size: u64,
    updated_at: i64,
) -> Result<()> {
    connection.execute(
        "INSERT INTO files_index (project_id, path, updated_at, size)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(project_id, path) DO UPDATE SET
             updated_at = excluded.updated_at,
             size = excluded.size",
        params![project_id, path, updated_at, size as i64],
    )?;
    Ok(())
}

pub fn max_event_seq(connection: &Connection) -> Result<u64> {
    let max: Option<i64> =
        connection.query_row("SELECT MAX(seq) FROM event_log", [], |row| row.get(0))?;
    Ok(max.unwrap_or(0) as u64)
}

pub fn insert_event(
    connection: &Connection,
    seq: u64,
    kind: &str,
    payload: &[u8],
    created_at: i64,
) -> Result<()> {
    connection.execute(
        "INSERT INTO event_log (seq, kind, payload, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![seq as i64, kind, payload, created_at],
    )?;
    Ok(())
}
