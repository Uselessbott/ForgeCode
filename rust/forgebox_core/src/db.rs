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

CREATE TABLE IF NOT EXISTS terminal_sessions (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL,
    cwd          TEXT NOT NULL,
    env_overrides_json TEXT NOT NULL DEFAULT '{}',
    last_used    INTEGER NOT NULL,
    was_running  INTEGER NOT NULL DEFAULT 0,
    exit_code    INTEGER
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
    let mut statement = connection.prepare(
        "SELECT p.id, p.name, p.path, p.created_at, COALESCE(SUM(f.size), 0)
         FROM projects p
         LEFT JOIN files_index f ON f.project_id = p.id
         GROUP BY p.id, p.name, p.path, p.created_at
         ORDER BY p.created_at, p.id",
    )?;
    let rows = statement.query_map([], |row| {
        let storage: i64 = row.get(4)?;
        Ok(Project {
            id: row.get(0)?,
            name: row.get(1)?,
            path: row.get(2)?,
            created_at: row.get(3)?,
            storage_bytes: storage.max(0) as u64,
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

pub fn delete_file_index_prefix(
    connection: &Connection,
    project_id: &str,
    path_prefix: &str,
) -> Result<()> {
    let like = format!(
        "{}/%",
        path_prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    connection.execute(
        "DELETE FROM files_index WHERE project_id = ?1 AND (path = ?2 OR path LIKE ?3 ESCAPE '\\')",
        params![project_id, path_prefix, like],
    )?;
    Ok(())
}

pub fn rename_file_index(
    connection: &Connection,
    project_id: &str,
    old_path: &str,
    new_path: &str,
) -> Result<()> {
    connection.execute(
        "UPDATE files_index SET path = ?3 WHERE project_id = ?1 AND path = ?2",
        params![project_id, old_path, new_path],
    )?;
    let like = format!(
        "{}/%",
        old_path
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let mut stmt = connection.prepare(
        "SELECT path, updated_at, size FROM files_index WHERE project_id = ?1 AND path LIKE ?2 ESCAPE '\\'",
    )?;
    let rows = stmt.query_map(params![project_id, like], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let children: Vec<(String, i64, i64)> = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    drop(stmt);
    for (child_path, updated_at, size) in children {
        let suffix = &child_path[old_path.len()..];
        let renamed = format!("{new_path}{suffix}");
        connection.execute(
            "DELETE FROM files_index WHERE project_id = ?1 AND path = ?2",
            params![project_id, child_path],
        )?;
        connection.execute(
            "INSERT OR REPLACE INTO files_index (project_id, path, updated_at, size) VALUES (?1, ?2, ?3, ?4)",
            params![project_id, renamed, updated_at, size],
        )?;
    }
    Ok(())
}

pub fn project_created_at(connection: &Connection, project_id: &str) -> Result<i64> {
    let ts: i64 = connection.query_row(
        "SELECT created_at FROM projects WHERE id = ?1",
        params![project_id],
        |row| row.get(0),
    )?;
    Ok(ts)
}

pub fn delete_project(connection: &Connection, project_id: &str) -> Result<()> {
    connection.execute("DELETE FROM projects WHERE id = ?1", params![project_id])?;
    Ok(())
}

pub fn rename_project(
    connection: &Connection,
    old_id: &str,
    new_id: &str,
    new_name: &str,
    new_path: &str,
) -> Result<()> {
    connection.execute(
        "UPDATE projects SET id = ?2, name = ?3, path = ?4 WHERE id = ?1",
        params![old_id, new_id, new_name, new_path],
    )?;
    connection.execute(
        "UPDATE files_index SET project_id = ?2 WHERE project_id = ?1",
        params![old_id, new_id],
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

#[derive(Clone, Debug)]
pub struct TerminalSessionRow {
    pub id: u64,
    pub name: String,
    pub cwd: String,
    pub env_overrides_json: String,
    pub last_used: i64,
    pub was_running: bool,
    pub exit_code: Option<i32>,
}

pub fn upsert_terminal_session(connection: &Connection, row: &TerminalSessionRow) -> Result<()> {
    connection.execute(
        "INSERT INTO terminal_sessions (id, name, cwd, env_overrides_json, last_used, was_running, exit_code)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
           name=excluded.name, cwd=excluded.cwd, env_overrides_json=excluded.env_overrides_json,
           last_used=excluded.last_used, was_running=excluded.was_running, exit_code=excluded.exit_code",
        params![
            row.id as i64,
            row.name,
            row.cwd,
            row.env_overrides_json,
            row.last_used,
            if row.was_running { 1i64 } else { 0i64 },
            row.exit_code.map(|v| v as i64),
        ],
    )?;
    Ok(())
}

pub fn list_terminal_sessions(connection: &Connection) -> Result<Vec<TerminalSessionRow>> {
    let mut stmt = connection.prepare(
        "SELECT id, name, cwd, env_overrides_json, last_used, was_running, exit_code
         FROM terminal_sessions ORDER BY last_used DESC, id DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        let was_running: i64 = row.get(5)?;
        let exit_code: Option<i64> = row.get(6)?;
        Ok(TerminalSessionRow {
            id: row.get::<_, i64>(0)? as u64,
            name: row.get(1)?,
            cwd: row.get(2)?,
            env_overrides_json: row.get(3)?,
            last_used: row.get(4)?,
            was_running: was_running != 0,
            exit_code: exit_code.map(|v| v as i32),
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub fn mark_terminal_session_running(
    connection: &Connection,
    id: u64,
    was_running: bool,
    exit_code: Option<i32>,
    last_used: i64,
) -> Result<()> {
    connection.execute(
        "UPDATE terminal_sessions SET was_running=?2, exit_code=?3, last_used=?4 WHERE id=?1",
        params![
            id as i64,
            if was_running { 1i64 } else { 0i64 },
            exit_code.map(|v| v as i64),
            last_used
        ],
    )?;
    Ok(())
}

pub fn clear_terminal_restore(connection: &Connection, id: u64) -> Result<()> {
    connection.execute(
        "UPDATE terminal_sessions SET was_running=0 WHERE id=?1",
        params![id as i64],
    )?;
    Ok(())
}

pub fn max_terminal_session_id(connection: &Connection) -> Result<u64> {
    let max: Option<i64> =
        connection.query_row("SELECT MAX(id) FROM terminal_sessions", [], |row| {
            row.get(0)
        })?;
    Ok(max.unwrap_or(0).max(0) as u64)
}

pub fn rename_terminal_session(
    connection: &Connection,
    id: u64,
    name: &str,
    last_used: i64,
) -> Result<()> {
    connection.execute(
        "UPDATE terminal_sessions SET name=?2, last_used=?3 WHERE id=?1",
        params![id as i64, name, last_used],
    )?;
    Ok(())
}
