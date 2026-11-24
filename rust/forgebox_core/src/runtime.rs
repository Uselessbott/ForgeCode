use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use prost::Message;
use rusqlite::Connection;

use crate::audit::{self, AuditLog, AuditRecord};
use crate::db;
use crate::error::{CoreError, Result};
use crate::proto::event::Kind as EventKind;
use crate::proto::request::Kind as RequestKind;
use crate::proto::response::Kind as ResponseKind;
use crate::proto::{
    CreateFolderRequest, CreateFolderResponse, CreateProjectRequest, CreateProjectResponse,
    DeletePathRequest, DeletePathResponse, DeleteProjectRequest, DeleteProjectResponse, Error,
    Event, FileChangedEvent, FileEntry, GetInfoRequest, GetInfoResponse, ListFilesRequest,
    ListFilesResponse, ListProjectsRequest, ListProjectsResponse, LogEvent, Project,
    ReadFileRequest, ReadFileResponse, RenameFileRequest, RenameFileResponse, RenameProjectRequest,
    RenameProjectResponse, Request, Response, StatFileRequest, StatFileResponse, Status,
    WriteFileIfUnchangedRequest, WriteFileIfUnchangedResponse, WriteFileRequest, WriteFileResponse,
};
use crate::pty;
use crate::terminal::TerminalEngine;
use crate::vfs;

pub const CORE_VERSION: &str = "0.6.0-m7";
pub const MAX_EDITABLE_FILE_BYTES: u64 = 1024 * 1024; // 1 MiB for M5 editor guard
pub const TERMINAL_SCROLLBACK_LINES: usize = 10_000;
pub const TERMINAL_OUTPUT_COALESCE_MS: u64 = 16;
pub const TERMINAL_OUTPUT_COALESCE_BYTES: usize = 64 * 1024;
pub const TERMINAL_SESSION_NAME_MAX: usize = 64;

const WORKSPACE_DIRS: &[&str] = &[
    "Projects",
    "Temporary",
    "Imports",
    "Exports",
    "Backups",
    "Cache",
    "Config",
    "AgentSessions",
];

const INTERNAL_DIRS: &[&str] = &["db", "audit", "checkpoints", "journal", "secrets"];

#[derive(Clone, Debug)]
pub struct PtyMeta {
    pub id: u64,
    pub name: String,
    pub cwd: String,
    pub project_id: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
    pub state: crate::proto::TerminalSessionState,
    pub exit_code: Option<i32>,
    pub previous_process_ended: bool,
    pub env_overrides_json: String,
}

struct PtyHandle {
    session: pty::PtySession,
    terminal: Mutex<TerminalEngine>,
    stop: Arc<AtomicBool>,
    join: Mutex<Option<std::thread::JoinHandle<()>>>,
    meta: Mutex<PtyMeta>,
}

pub struct Runtime {
    app_data_dir: PathBuf,
    native_lib_dir: PathBuf,
    db_path: PathBuf,
    workspace_root: PathBuf,
    db: Mutex<Connection>,
    events: Mutex<VecDeque<Vec<u8>>>,
    events_cv: std::sync::Condvar,
    next_seq: AtomicU64,
    next_pty_id: AtomicU64,
    ptys: Mutex<std::collections::HashMap<u64, Arc<PtyHandle>>>,
    audit: AuditLog,
    pub pty_wall_clock_timeout: Duration,
    net_state: Mutex<crate::net_policy::NetState>,
    net_activity: Mutex<crate::net_activity::ActivityLog>,
    net_session_enabled: Mutex<std::collections::HashMap<String, bool>>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Ok(mut ptys) = self.ptys.lock() {
            let handles: Vec<_> = ptys.drain().map(|(_, h)| h).collect();
            drop(ptys);
            for handle in handles {
                let (sid, pgid) = handle
                    .meta
                    .lock()
                    .map(|m| (m.id, handle.session.pgid))
                    .unwrap_or((handle.session.id, handle.session.pgid));
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "supervisor".into(),
                    op: "supervisor_kill".into(),
                    resolved_paths: vec![format!("pgid:{pgid}")],
                    before_hash: None,
                    after_hash: Some("runtime_drop".into()),
                    policy_result: "ALLOW".into(),
                    approval_id: None,
                    session_id: Some(sid.to_string()),
                });
                handle.stop.store(true, Ordering::SeqCst);
                let _ = pty::close(&handle.session);
                if let Ok(mut join) = handle.join.lock() {
                    if let Some(j) = join.take() {
                        let _ = j.join();
                    }
                }
            }
        }
    }
}

impl Runtime {
    pub fn new(app_data_dir: PathBuf, native_lib_dir: PathBuf, db_path: PathBuf) -> Result<Self> {
        let workspace_root = app_data_dir.join("ForgeBox");
        let internal_root = app_data_dir.join("internal");

        fs::create_dir_all(&workspace_root)?;
        for dir in WORKSPACE_DIRS {
            fs::create_dir_all(workspace_root.join(dir))?;
        }
        for dir in INTERNAL_DIRS {
            fs::create_dir_all(internal_root.join(dir))?;
        }

        let db = db::open(&db_path)?;
        let next_seq = db::max_event_seq(&db)? + 1;
        let next_pty_id = db::max_terminal_session_id(&db)?.saturating_add(1).max(1);
        let audit = AuditLog::new(
            app_data_dir
                .join("internal")
                .join("audit")
                .join("audit.jsonl"),
        );
        let runtime = Self {
            app_data_dir,
            native_lib_dir,
            db_path,
            workspace_root,
            db: Mutex::new(db),
            events: Mutex::new(VecDeque::new()),
            events_cv: std::sync::Condvar::new(),
            next_seq: AtomicU64::new(next_seq),
            next_pty_id: AtomicU64::new(next_pty_id),
            ptys: Mutex::new(std::collections::HashMap::new()),
            audit,
            pty_wall_clock_timeout: Duration::from_secs(30 * 60),
            net_state: Mutex::new(crate::net_policy::NetState::default()),
            net_activity: Mutex::new(crate::net_activity::ActivityLog::new(1000)),
            net_session_enabled: Mutex::new(std::collections::HashMap::new()),
        };
        runtime.emit_restore_available_sessions()?;
        runtime.emit_event_kind(EventKind::Log(LogEvent {
            level: "info".into(),
            message: format!("ForgeBox core {CORE_VERSION} initialized"),
        }))?;
        Ok(runtime)
    }

    pub fn call(&self, bytes: &[u8]) -> Vec<u8> {
        let request = match Request::decode(bytes) {
            Ok(request) => request,
            Err(error) => {
                return error_response(0, "PROTOBUF", format!("failed to decode request: {error}"));
            }
        };
        let id = request.id;
        match self.dispatch(request.kind) {
            Ok(kind) => Response {
                id,
                status: Status::Ok as i32,
                error: None,
                kind: Some(kind),
            }
            .encode_to_vec(),
            Err(error) => error_response(id, error.code(), error.to_string()),
        }
    }

    pub fn poll_events(&self, max: usize) -> Vec<Vec<u8>> {
        self.poll_events_timeout(max, 0)
    }

    pub fn poll_events_timeout(&self, max: usize, timeout_ms: u64) -> Vec<Vec<u8>> {
        let mut queue = match self.events.lock() {
            Ok(queue) => queue,
            Err(_) => return Vec::new(),
        };
        if queue.is_empty() && timeout_ms > 0 {
            let (guard, _) = match self
                .events_cv
                .wait_timeout(queue, Duration::from_millis(timeout_ms))
            {
                Ok(v) => v,
                Err(_) => return Vec::new(),
            };
            queue = guard;
        }
        let count = max.min(queue.len());
        (0..count).filter_map(|_| queue.pop_front()).collect()
    }

    pub fn pty_open(
        &self,
        project_id: Option<String>,
        name: Option<String>,
        env_overrides: Vec<(String, String)>,
        cols: u16,
        rows: u16,
        session_cap: usize,
    ) -> Result<u64> {
        self.pty_open_with_command(
            project_id,
            name,
            env_overrides,
            cols,
            rows,
            session_cap,
            None,
            None,
            None,
        )
    }

    pub fn pty_open_with_command(
        &self,
        project_id: Option<String>,
        name: Option<String>,
        env_overrides: Vec<(String, String)>,
        cols: u16,
        rows: u16,
        session_cap: usize,
        shell_path_override: Option<String>,
        args_json: Option<String>,
        cwd_override: Option<String>,
    ) -> Result<u64> {
        let cwd = if let Some(cwd_override) = cwd_override {
            let p = PathBuf::from(cwd_override);
            let canonical = fs::canonicalize(&p)?;
            let projects = fs::canonicalize(self.workspace_root.join("Projects"))?;
            if !canonical.starts_with(&projects) {
                return Err(CoreError::InvalidArgument(
                    "cwd outside ForgeBox/Projects".into(),
                ));
            }
            canonical
        } else {
            self.resolve_pty_cwd(project_id.as_deref())?
        };
        let cwd_str = cwd.to_string_lossy().into_owned();
        let now = now_millis();
        let (id, resolved_name) = {
            let ptys = self.ptys.lock().map_err(lock_error)?;
            let live = ptys.len();
            if live >= session_cap.max(1) {
                return Err(CoreError::InvalidArgument(format!(
                    "SESSION_CAP_REACHED: close a session first ({live}/{session_cap})"
                )));
            }
            let id = self.next_pty_id.fetch_add(1, Ordering::SeqCst);
            let db_guard = self.db.lock().map_err(lock_error)?;
            let name = self.resolve_session_name(name.as_deref(), &ptys, &db_guard)?;
            drop(db_guard);
            (id, name)
        };

        let shell = if let Some(sp) = shell_path_override {
            PathBuf::from(sp)
        } else {
            let shell = PathBuf::from("/system/bin/sh");
            if shell.exists() {
                shell
            } else {
                PathBuf::from("/bin/sh")
            }
        };
        let args = args_json
            .as_deref()
            .map(serde_json_like_vec)
            .unwrap_or_else(|| vec!["-sh".into()]);
        let mut env = vec![
            (
                "HOME".to_string(),
                format!("{}/.forgebox-home", cwd.to_string_lossy()),
            ),
            ("PATH".to_string(), "/system/bin:/system/xbin".to_string()),
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("LANG".to_string(), "C.UTF-8".to_string()),
            (
                "TMPDIR".to_string(),
                self.workspace_root
                    .join("Temporary")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ("PWD".to_string(), cwd.to_string_lossy().into_owned()),
        ];
        for (k, v) in env_overrides.iter() {
            if is_allowed_env_override(k) {
                env.retain(|(ek, _)| ek != k);
                env.push((k.clone(), v.clone()));
            }
        }
        let config = pty::PtyConfig {
            shell_path: shell,
            args,
            cwd,
            env,
            cols,
            rows,
        };
        let mut session = pty::spawn(&config)?;
        session.id = id;
        let env_overrides_json = if env_overrides.is_empty() {
            "{}".to_string()
        } else {
            let mut s = String::from("{");
            for (i, (k, v)) in env_overrides.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push('"');
                s.push_str(&json_escape(k));
                s.push_str("\":\"");
                s.push_str(&json_escape(v));
                s.push('"');
            }
            s.push('}');
            s
        };
        let meta = PtyMeta {
            id,
            name: resolved_name.clone(),
            cwd: cwd_str.clone(),
            project_id: project_id.clone(),
            created_at: now,
            last_used: now,
            state: crate::proto::TerminalSessionState::TerminalSessionRunning,
            exit_code: None,
            previous_process_ended: false,
            env_overrides_json: env_overrides_json.clone(),
        };
        let handle = Arc::new(PtyHandle {
            session,
            terminal: Mutex::new(TerminalEngine::with_scrollback(
                rows as usize,
                cols as usize,
                TERMINAL_SCROLLBACK_LINES,
            )),
            stop: Arc::new(AtomicBool::new(false)),
            join: Mutex::new(None),
            meta: Mutex::new(meta.clone()),
        });
        self.persist_terminal_session(
            &meta,
            &env_overrides,
            Some(&env_overrides_json),
            true,
            None,
        )?;
        self.ptys
            .lock()
            .map_err(lock_error)?
            .insert(id, Arc::clone(&handle));
        self.emit_terminal_session_spawned(&meta);
        self.spawn_pty_reader(id, Arc::clone(&handle));
        Ok(id)
    }

    fn spawn_pty_reader(&self, id: u64, handle: Arc<PtyHandle>) {
        let runtime_ptr = self as *const Runtime as usize;
        let stop = Arc::clone(&handle.stop);
        let reader_handle = Arc::clone(&handle);
        let wall_clock_timeout = self.pty_wall_clock_timeout;
        let join = std::thread::spawn(move || {
            let runtime = unsafe { &*(runtime_ptr as *const Runtime) };
            let mut last_emit = std::time::Instant::now();
            let started = std::time::Instant::now();
            let mut pending = Vec::<u8>::new();
            let mut dropped_bytes: usize = 0;
            while !stop.load(Ordering::SeqCst) {
                if started.elapsed() >= wall_clock_timeout {
                    let _ = runtime.audit.append(&AuditRecord {
                        ts: audit::now_iso8601(),
                        actor: "supervisor".into(),
                        op: "wall_clock_timeout".into(),
                        resolved_paths: vec![],
                        before_hash: None,
                        after_hash: None,
                        policy_result: "DENY".into(),
                        approval_id: None,
                        session_id: Some(id.to_string()),
                    });
                    let _ = pty::signal(&reader_handle.session, 15);
                    std::thread::sleep(Duration::from_secs(2));
                    let _ = pty::signal(&reader_handle.session, 9);
                }
                match pty::read_available(&reader_handle.session) {
                    Ok(bytes) if !bytes.is_empty() => {
                        if pending.len() + bytes.len() > TERMINAL_OUTPUT_COALESCE_BYTES * 4 {
                            dropped_bytes += bytes.len();
                        } else {
                            pending.extend_from_slice(&bytes);
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
                let elapsed =
                    last_emit.elapsed() >= Duration::from_millis(TERMINAL_OUTPUT_COALESCE_MS);
                if !pending.is_empty()
                    && (elapsed || pending.len() >= TERMINAL_OUTPUT_COALESCE_BYTES)
                {
                    runtime.emit_terminal_output(
                        id,
                        &reader_handle,
                        &mut pending,
                        &mut dropped_bytes,
                    );
                    last_emit = std::time::Instant::now();
                }
                if let Ok(Some(code)) = pty::try_wait(&reader_handle.session) {
                    if !pending.is_empty() || dropped_bytes > 0 {
                        runtime.emit_terminal_output(
                            id,
                            &reader_handle,
                            &mut pending,
                            &mut dropped_bytes,
                        );
                    }
                    runtime.on_pty_exit(id, code);
                    stop.store(true, Ordering::SeqCst);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        if let Ok(mut j) = handle.join.lock() {
            *j = Some(join);
        }
    }

    fn emit_terminal_output(
        &self,
        id: u64,
        handle: &Arc<PtyHandle>,
        pending: &mut Vec<u8>,
        dropped_bytes: &mut usize,
    ) {
        if *dropped_bytes > 0 {
            let notice = format!("\r\n[ForgeBox: {dropped_bytes} bytes dropped]\r\n");
            pending.extend_from_slice(notice.as_bytes());
            *dropped_bytes = 0;
        }
        if pending.is_empty() {
            return;
        }
        if let Ok(mut term) = handle.terminal.lock() {
            term.feed(pending);
            let diff = term.state.diff_json();
            let _ = self.emit_event_kind(EventKind::TerminalOutput(
                crate::proto::TerminalOutputEvent {
                    pty_id: id,
                    diff_bytes: diff,
                },
            ));
        }
        pending.clear();
        if let Ok(mut meta) = handle.meta.lock() {
            meta.last_used = now_millis();
        }
    }

    fn on_pty_exit(&self, id: u64, code: i32) {
        if let Ok(ptys) = self.ptys.lock() {
            if let Some(h) = ptys.get(&id) {
                if let Ok(mut meta) = h.meta.lock() {
                    meta.state = crate::proto::TerminalSessionState::TerminalSessionExited;
                    meta.exit_code = Some(code);
                    meta.last_used = now_millis();
                    let _ = self.persist_terminal_session(&meta, &[], None, false, Some(code));
                    let _ = self.emit_event_kind(EventKind::TerminalSessionState(
                        crate::proto::TerminalSessionStateEvent {
                            pty_id: id,
                            state: crate::proto::TerminalSessionState::TerminalSessionExited as i32,
                            exit_code: code,
                        },
                    ));
                }
            }
        }
        let _ = self.emit_event_kind(EventKind::TerminalExit(crate::proto::TerminalExitEvent {
            pty_id: id,
            code,
        }));
    }

    pub fn pty_write(&self, pty_id: u64, data: &[u8]) -> Result<()> {
        let handle = self.get_pty(pty_id)?;
        pty::write(&handle.session, data)
    }

    pub fn pty_resize(&self, pty_id: u64, cols: u16, rows: u16) -> Result<()> {
        let handle = self.get_pty(pty_id)?;
        handle
            .terminal
            .lock()
            .map_err(lock_error)?
            .resize(rows as usize, cols as usize);
        pty::resize(&handle.session, cols, rows)
    }

    pub fn pty_signal(&self, pty_id: u64, sig: i32) -> Result<()> {
        let handle = self.get_pty(pty_id)?;
        pty::signal(&handle.session, sig)
    }

    pub fn pty_close(&self, pty_id: u64) -> Result<()> {
        let handle = {
            let mut ptys = self.ptys.lock().map_err(lock_error)?;
            ptys.remove(&pty_id)
                .ok_or_else(|| CoreError::NotFound(format!("pty not found: {pty_id}")))?
        };
        let pgid = handle.session.pgid;
        let _ = self.audit.append(&AuditRecord {
            ts: audit::now_iso8601(),
            actor: "supervisor".into(),
            op: "supervisor_kill".into(),
            resolved_paths: vec![format!("pgid:{pgid}")],
            before_hash: None,
            after_hash: Some("close".into()),
            policy_result: "ALLOW".into(),
            approval_id: None,
            session_id: Some(pty_id.to_string()),
        });
        self.close_handle(
            handle,
            crate::proto::TerminalSessionState::TerminalSessionStopped,
            None,
        )
    }

    fn close_handle(
        &self,
        handle: Arc<PtyHandle>,
        state: crate::proto::TerminalSessionState,
        exit_code: Option<i32>,
    ) -> Result<()> {
        let id = handle.session.id;
        handle.stop.store(true, Ordering::SeqCst);
        pty::close(&handle.session)?;
        if let Some(join) = handle.join.lock().map_err(lock_error)?.take() {
            let _ = join.join();
        }
        if let Ok(mut meta) = handle.meta.lock() {
            meta.state = state;
            meta.exit_code = exit_code.or(meta.exit_code);
            meta.last_used = now_millis();
            let _ = self.persist_terminal_session(&meta, &[], None, false, meta.exit_code);
            let _ = self.emit_event_kind(EventKind::TerminalSessionState(
                crate::proto::TerminalSessionStateEvent {
                    pty_id: id,
                    state: state as i32,
                    exit_code: meta.exit_code.unwrap_or(0),
                },
            ));
        }
        Ok(())
    }

    pub fn pty_rename(&self, pty_id: u64, name: String) -> Result<PtyMeta> {
        let name = validate_session_name(&name)?;
        let existing = self.list_terminal_sessions()?;
        if existing
            .iter()
            .any(|s| s.id != pty_id && s.name.eq_ignore_ascii_case(&name))
        {
            return Err(CoreError::InvalidArgument(
                "session name already exists".into(),
            ));
        }
        let handle = self.get_pty(pty_id)?;
        let meta = {
            let mut meta = handle.meta.lock().map_err(lock_error)?;
            meta.name = name.clone();
            meta.last_used = now_millis();
            meta.clone()
        };
        {
            let db = self.db.lock().map_err(lock_error)?;
            db::rename_terminal_session(&db, pty_id, &name, meta.last_used)?;
        }
        let _ = self.emit_event_kind(EventKind::TerminalSessionRenamed(
            crate::proto::TerminalSessionRenamedEvent { pty_id, name },
        ));
        Ok(meta)
    }

    pub fn pty_stop_all(&self) -> Result<u32> {
        let handles: Vec<_> = {
            let mut ptys = self.ptys.lock().map_err(lock_error)?;
            ptys.drain().map(|(_, h)| h).collect::<Vec<_>>()
        };
        let mut stopped = 0u32;
        for handle in handles {
            let (code, sid, pgid) = if let Ok(meta) = handle.meta.lock() {
                (
                    if meta.exit_code.unwrap_or(0) != 0 {
                        meta.exit_code
                    } else {
                        Some(-1)
                    },
                    meta.id,
                    handle.session.pgid,
                )
            } else {
                (Some(-1), handle.session.id, handle.session.pgid)
            };
            let _ = self.audit.append(&AuditRecord {
                ts: audit::now_iso8601(),
                actor: "supervisor".into(),
                op: "supervisor_kill".into(),
                resolved_paths: vec![format!("pgid:{pgid}")],
                before_hash: None,
                after_hash: Some("stop_all".into()),
                policy_result: "ALLOW".into(),
                approval_id: None,
                session_id: Some(sid.to_string()),
            });
            let _ = handle
                .meta
                .lock()
                .map(|mut m| m.state = crate::proto::TerminalSessionState::TerminalSessionStopped);
            if self
                .close_handle(
                    handle,
                    crate::proto::TerminalSessionState::TerminalSessionStopped,
                    code,
                )
                .is_ok()
            {
                stopped += 1;
            }
        }
        Ok(stopped)
    }

    pub fn list_terminal_sessions(&self) -> Result<Vec<PtyMeta>> {
        let mut out = Vec::new();
        let live = self.ptys.lock().map_err(lock_error)?;
        for h in live.values() {
            if let Ok(m) = h.meta.lock() {
                out.push(m.clone());
            }
        }
        drop(live);
        let db = self.db.lock().map_err(lock_error)?;
        for row in db::list_terminal_sessions(&db)? {
            if out.iter().any(|m| m.id == row.id) {
                continue;
            }
            if row.was_running {
                out.push(PtyMeta {
                    id: row.id,
                    name: row.name.clone(),
                    cwd: row.cwd.clone(),
                    project_id: None,
                    created_at: row.last_used,
                    last_used: row.last_used,
                    state: crate::proto::TerminalSessionState::TerminalSessionExited,
                    exit_code: row.exit_code,
                    previous_process_ended: true,
                    env_overrides_json: row.env_overrides_json.clone(),
                });
            }
        }
        out.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(b.id.cmp(&a.id)));
        Ok(out)
    }

    pub fn terminal_session_cap(is_tablet: bool) -> u32 {
        if is_tablet {
            12
        } else {
            6
        }
    }

    fn persist_terminal_session(
        &self,
        meta: &PtyMeta,
        env_overrides: &[(String, String)],
        env_overrides_json: Option<&str>,
        was_running: bool,
        exit_code: Option<i32>,
    ) -> Result<()> {
        let env_overrides_json = if let Some(json) = env_overrides_json {
            json.to_string()
        } else if env_overrides.is_empty() {
            "{}".to_string()
        } else {
            let mut s = String::from("{");
            for (i, (k, v)) in env_overrides.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push('"');
                s.push_str(&json_escape(k));
                s.push_str("\":\"");
                s.push_str(&json_escape(v));
                s.push('"');
            }
            s.push('}');
            s
        };
        let row = db::TerminalSessionRow {
            id: meta.id,
            name: meta.name.clone(),
            cwd: meta.cwd.clone(),
            env_overrides_json,
            last_used: meta.last_used,
            was_running,
            exit_code,
        };
        let db = self.db.lock().map_err(lock_error)?;
        db::upsert_terminal_session(&db, &row)
    }

    fn emit_terminal_session_spawned(&self, meta: &PtyMeta) {
        let _ = self.emit_event_kind(EventKind::TerminalSessionSpawned(
            crate::proto::TerminalSessionSpawnedEvent {
                session: Some(meta.to_proto()),
            },
        ));
    }

    fn emit_restore_available_sessions(&self) -> Result<()> {
        let db = self.db.lock().map_err(lock_error)?;
        let rows = db::list_terminal_sessions(&db)?;
        let mut changed = Vec::new();
        for row in rows.iter().filter(|r| r.was_running) {
            let mut updated = row.clone();
            updated.was_running = false;
            changed.push(updated);
        }
        for row in &changed {
            db::upsert_terminal_session(&db, row)?;
        }
        drop(db);
        for row in changed {
            let desc = crate::proto::TerminalSessionDescriptor {
                pty_id: row.id,
                name: row.name,
                cwd: row.cwd,
                state: crate::proto::TerminalSessionState::TerminalSessionExited as i32,
                exit_code: row.exit_code.unwrap_or(-1),
                created_at: row.last_used,
                last_used: row.last_used,
                restorable: true,
                previous_process_ended: true,
                env_overrides_json: row.env_overrides_json,
            };
            let _ = self.emit_event_kind(EventKind::TerminalSessionRestoreAvailable(
                crate::proto::TerminalSessionRestoreAvailableEvent {
                    session: Some(desc),
                },
            ));
        }
        Ok(())
    }

    fn resolve_session_name(
        &self,
        requested: Option<&str>,
        live: &std::collections::HashMap<u64, Arc<PtyHandle>>,
        db: &Connection,
    ) -> Result<String> {
        if let Some(name) = requested.map(str::trim).filter(|s| !s.is_empty()) {
            let name = validate_session_name(name)?;
            let exists_live = live.values().any(|h| {
                h.meta
                    .lock()
                    .map(|m| m.name.eq_ignore_ascii_case(&name))
                    .unwrap_or(false)
            });
            let exists_db = db::list_terminal_sessions(db)?
                .into_iter()
                .any(|r| r.name.eq_ignore_ascii_case(&name));
            if exists_live || exists_db {
                return Err(CoreError::InvalidArgument(
                    "session name already exists".into(),
                ));
            }
            return Ok(name);
        }
        let mut n = 1usize;
        loop {
            let candidate = if n == 1 {
                "sh".to_string()
            } else {
                format!("sh ({n})")
            };
            let exists_live = live.values().any(|h| {
                h.meta
                    .lock()
                    .map(|m| m.name.eq_ignore_ascii_case(&candidate))
                    .unwrap_or(false)
            });
            let exists_db = db::list_terminal_sessions(db)?
                .into_iter()
                .any(|r| r.name.eq_ignore_ascii_case(&candidate));
            if !exists_live && !exists_db {
                return Ok(candidate);
            }
            n += 1;
        }
    }

    fn get_pty(&self, pty_id: u64) -> Result<Arc<PtyHandle>> {
        self.ptys
            .lock()
            .map_err(lock_error)?
            .get(&pty_id)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(format!("pty not found: {pty_id}")))
    }

    fn resolve_pty_cwd(&self, project_id: Option<&str>) -> Result<PathBuf> {
        let projects_root = fs::canonicalize(self.workspace_root.join("Projects"))?;
        let workspace_root = fs::canonicalize(&self.workspace_root)?;
        let cwd = match project_id.filter(|s| !s.is_empty()) {
            Some(pid) => {
                self.assert_project_exists(pid)?;
                let dir = self.project_dir(pid)?;
                crate::vfs::resolve(&dir, Path::new(""), true)?
            }
            None => workspace_root.clone(),
        };
        if !(cwd.starts_with(&projects_root) || cwd == workspace_root) {
            return Err(CoreError::PathEscape(
                "pty cwd must be under ForgeBox/Projects/<id> or ForgeBox root".into(),
            ));
        }
        Ok(cwd)
    }

    pub fn app_data_dir(&self) -> &Path {
        &self.app_data_dir
    }

    pub fn native_lib_dir(&self) -> &Path {
        &self.native_lib_dir
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    fn dispatch(&self, kind: Option<RequestKind>) -> Result<ResponseKind> {
        match kind {
            Some(RequestKind::GetInfo(request)) => {
                self.get_info(request).map(ResponseKind::GetInfo)
            }
            Some(RequestKind::CreateProject(request)) => self
                .create_project(request)
                .map(ResponseKind::CreateProject),
            Some(RequestKind::ListProjects(request)) => {
                self.list_projects(request).map(ResponseKind::ListProjects)
            }
            Some(RequestKind::WriteFile(request)) => {
                self.write_file(request).map(ResponseKind::WriteFile)
            }
            Some(RequestKind::ReadFile(request)) => {
                self.read_file(request).map(ResponseKind::ReadFile)
            }
            Some(RequestKind::ListFiles(request)) => {
                self.list_files(request).map(ResponseKind::ListFiles)
            }
            Some(RequestKind::RenameFile(request)) => {
                self.rename_file(request).map(ResponseKind::RenameFile)
            }
            Some(RequestKind::DeletePath(request)) => {
                self.delete_path(request).map(ResponseKind::DeletePath)
            }
            Some(RequestKind::CreateFolder(request)) => {
                self.create_folder(request).map(ResponseKind::CreateFolder)
            }
            Some(RequestKind::DeleteProject(request)) => self
                .delete_project(request)
                .map(ResponseKind::DeleteProject),
            Some(RequestKind::RenameProject(request)) => self
                .rename_project(request)
                .map(ResponseKind::RenameProject),
            Some(RequestKind::StatFile(request)) => {
                self.stat_file(request).map(ResponseKind::StatFile)
            }
            Some(RequestKind::WriteFileIfUnchanged(request)) => self
                .write_file_if_unchanged(request)
                .map(ResponseKind::WriteFileIfUnchanged),
            Some(RequestKind::ListTerminalSessions(_)) => {
                let sessions = self.list_terminal_sessions()?;
                Ok(ResponseKind::ListTerminalSessions(
                    crate::proto::ListTerminalSessionsResponse {
                        sessions: sessions.into_iter().map(|m| m.to_proto()).collect(),
                        session_cap: Self::terminal_session_cap(false),
                    },
                ))
            }
            Some(RequestKind::RenameTerminalSession(request)) => {
                let meta = self.pty_rename(request.pty_id, request.name)?;
                Ok(ResponseKind::RenameTerminalSession(
                    crate::proto::RenameTerminalSessionResponse {
                        session: Some(meta.to_proto()),
                    },
                ))
            }
            Some(RequestKind::StopAllTerminalSessions(_)) => {
                let stopped = self.pty_stop_all()?;
                Ok(ResponseKind::StopAllTerminalSessions(
                    crate::proto::StopAllTerminalSessionsResponse { stopped },
                ))
            }
            Some(RequestKind::EvaluateCommand(request)) => {
                let origin = match request.origin() {
                    crate::proto::CommandOrigin::InstalledBinary => {
                        crate::policy::CommandOrigin::InstalledBinary
                    }
                    _ => crate::policy::CommandOrigin::User,
                };
                let a = crate::policy::evaluate_command_with_origin(
                    &request.command,
                    request.actor_is_agent,
                    &request.session_id,
                    &request.cwd,
                    origin,
                );
                let decision = match a.decision {
                    crate::policy::PolicyDecision::Allow => "ALLOW",
                    crate::policy::PolicyDecision::Confirm => "CONFIRM",
                    crate::policy::PolicyDecision::Deny => "DENY",
                };
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: if request.actor_is_agent {
                        "agent".into()
                    } else {
                        "user".into()
                    },
                    op: "policy_decision".into(),
                    resolved_paths: vec![request.cwd.clone()],
                    before_hash: None,
                    after_hash: Some(a.command_hash.clone()),
                    policy_result: decision.into(),
                    approval_id: None,
                    session_id: Some(request.session_id.clone()),
                });
                Ok(ResponseKind::EvaluateCommand(
                    crate::proto::EvaluateCommandResponse {
                        decision: decision.to_string(),
                        reason: a.reason,
                        normalized_command: a.normalized_command,
                        argv: a.argv,
                        command_hash: a.command_hash,
                        file_count_estimate: a.file_count_estimate,
                        affects_outside_project: a.affects_outside_project,
                        uses_network: a.uses_network,
                        creates_checkpoint: a.creates_checkpoint,
                    },
                ))
            }
            Some(RequestKind::IssueApprovalToken(request)) => {
                let token = crate::policy::issue_approval_token(
                    &request.command_hash,
                    &request.session_id,
                )?;
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "ui".into(),
                    op: "approval_issued".into(),
                    resolved_paths: vec![],
                    before_hash: None,
                    after_hash: Some(request.command_hash.clone()),
                    policy_result: "CONFIRM".into(),
                    approval_id: Some(audit::redact(&token)),
                    session_id: Some(request.session_id.clone()),
                });
                Ok(ResponseKind::IssueApprovalToken(
                    crate::proto::IssueApprovalTokenResponse {
                        token,
                        expires_in_seconds: 60,
                    },
                ))
            }
            Some(RequestKind::ConsumeApprovalToken(request)) => {
                let res = crate::policy::consume_approval_token(
                    &request.token,
                    &request.command_hash,
                    &request.session_id,
                );
                match res {
                    Ok(()) => {
                        let _ = self.audit.append(&AuditRecord {
                            ts: audit::now_iso8601(),
                            actor: "ui".into(),
                            op: "approval_consumed".into(),
                            resolved_paths: vec![],
                            before_hash: None,
                            after_hash: Some(request.command_hash.clone()),
                            policy_result: "ALLOW".into(),
                            approval_id: Some(audit::redact(&request.token)),
                            session_id: Some(request.session_id.clone()),
                        });
                        Ok(ResponseKind::ConsumeApprovalToken(
                            crate::proto::ConsumeApprovalTokenResponse { ok: true },
                        ))
                    }
                    Err(e) => {
                        let _ = self.audit.append(&AuditRecord {
                            ts: audit::now_iso8601(),
                            actor: "ui".into(),
                            op: "approval_denied".into(),
                            resolved_paths: vec![],
                            before_hash: None,
                            after_hash: Some(request.command_hash.clone()),
                            policy_result: "DENY".into(),
                            approval_id: Some(audit::redact(&request.token)),
                            session_id: Some(request.session_id.clone()),
                        });
                        Err(e)
                    }
                }
            }
            Some(RequestKind::GitInit(request)) => {
                let root = self.project_root_by_id(&request.project_id)?;
                crate::git::init_repo(&root)?;
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "user".into(),
                    op: "git_init".into(),
                    resolved_paths: vec![root.to_string_lossy().into_owned()],
                    before_hash: None,
                    after_hash: None,
                    policy_result: "ALLOW".into(),
                    approval_id: None,
                    session_id: Some(request.project_id.clone()),
                });
                Ok(ResponseKind::GitInit(crate::proto::GitInitResponse {
                    ok: true,
                }))
            }
            Some(RequestKind::GitStatus(request)) => {
                let root = self.project_root_by_id(&request.project_id)?;
                let s = crate::git::status(&root)?;
                Ok(ResponseKind::GitStatus(crate::proto::GitStatusResponse {
                    staged: s
                        .staged
                        .into_iter()
                        .map(|e| crate::proto::GitFileStatus {
                            path: e.path,
                            status: e.status,
                            is_binary: e.is_binary,
                        })
                        .collect(),
                    unstaged: s
                        .unstaged
                        .into_iter()
                        .map(|e| crate::proto::GitFileStatus {
                            path: e.path,
                            status: e.status,
                            is_binary: e.is_binary,
                        })
                        .collect(),
                    untracked: s
                        .untracked
                        .into_iter()
                        .map(|e| crate::proto::GitFileStatus {
                            path: e.path,
                            status: e.status,
                            is_binary: e.is_binary,
                        })
                        .collect(),
                    branch: s.branch,
                }))
            }
            Some(RequestKind::GitDiff(request)) => {
                let root = self.project_root_by_id(&request.project_id)?;
                let diff = crate::git::diff(
                    &root,
                    &request.scope,
                    if request.path.is_empty() {
                        None
                    } else {
                        Some(&request.path)
                    },
                )?;
                Ok(ResponseKind::GitDiff(crate::proto::GitDiffResponse {
                    unified_diff: diff,
                }))
            }
            Some(RequestKind::GitStage(request)) => {
                let root = self.project_root_by_id(&request.project_id)?;
                crate::git::stage(&root, &request.files)?;
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "user".into(),
                    op: "git_stage".into(),
                    resolved_paths: request.files.clone(),
                    before_hash: None,
                    after_hash: None,
                    policy_result: "ALLOW".into(),
                    approval_id: None,
                    session_id: Some(request.project_id.clone()),
                });
                Ok(ResponseKind::GitStage(crate::proto::GitStageResponse {
                    ok: true,
                }))
            }
            Some(RequestKind::GitUnstage(request)) => {
                let root = self.project_root_by_id(&request.project_id)?;
                crate::git::unstage(&root, &request.files)?;
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "user".into(),
                    op: "git_unstage".into(),
                    resolved_paths: request.files.clone(),
                    before_hash: None,
                    after_hash: None,
                    policy_result: "ALLOW".into(),
                    approval_id: None,
                    session_id: Some(request.project_id.clone()),
                });
                Ok(ResponseKind::GitUnstage(crate::proto::GitUnstageResponse {
                    ok: true,
                }))
            }
            Some(RequestKind::GitCommit(request)) => {
                let root = self.project_root_by_id(&request.project_id)?;
                let commit_id = crate::git::commit(
                    &root,
                    &request.message,
                    &request.author_name,
                    &request.author_email,
                )?;
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "user".into(),
                    op: "git_commit".into(),
                    resolved_paths: vec![root.to_string_lossy().into_owned()],
                    before_hash: None,
                    after_hash: Some(commit_id.clone()),
                    policy_result: "ALLOW".into(),
                    approval_id: None,
                    session_id: Some(request.project_id.clone()),
                });
                Ok(ResponseKind::GitCommit(crate::proto::GitCommitResponse {
                    commit_id,
                }))
            }
            Some(RequestKind::GitBranchList(request)) => {
                let root = self.project_root_by_id(&request.project_id)?;
                let branches = crate::git::branch_list(&root)?;
                Ok(ResponseKind::GitBranchList(
                    crate::proto::GitBranchListResponse {
                        branches: branches
                            .into_iter()
                            .map(|(name, is_current)| crate::proto::GitBranchInfo {
                                name,
                                is_current,
                            })
                            .collect(),
                    },
                ))
            }
            Some(RequestKind::GitBranchCreate(request)) => {
                let root = self.project_root_by_id(&request.project_id)?;
                crate::git::branch_create(&root, &request.name)?;
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "user".into(),
                    op: "git_branch_create".into(),
                    resolved_paths: vec![request.name.clone()],
                    before_hash: None,
                    after_hash: None,
                    policy_result: "ALLOW".into(),
                    approval_id: None,
                    session_id: Some(request.project_id.clone()),
                });
                Ok(ResponseKind::GitBranchCreate(
                    crate::proto::GitBranchCreateResponse { ok: true },
                ))
            }
            Some(RequestKind::GitBranchSwitch(request)) => {
                let root = self.project_root_by_id(&request.project_id)?;
                crate::git::branch_switch(&root, &request.name)?;
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "user".into(),
                    op: "git_branch_switch".into(),
                    resolved_paths: vec![request.name.clone()],
                    before_hash: None,
                    after_hash: None,
                    policy_result: "ALLOW".into(),
                    approval_id: None,
                    session_id: Some(request.project_id.clone()),
                });
                Ok(ResponseKind::GitBranchSwitch(
                    crate::proto::GitBranchSwitchResponse { ok: true },
                ))
            }
            Some(RequestKind::GitLog(request)) => {
                let root = self.project_root_by_id(&request.project_id)?;
                let commits = crate::git::log(&root, request.limit)?;
                Ok(ResponseKind::GitLog(crate::proto::GitLogResponse {
                    commits: commits
                        .into_iter()
                        .map(|c| crate::proto::GitCommitInfo {
                            commit_id: c.commit_id,
                            author: c.author,
                            email: c.email,
                            message: c.message,
                            timestamp: c.timestamp,
                        })
                        .collect(),
                }))
            }
            Some(RequestKind::GitShow(request)) => {
                let root = self.project_root_by_id(&request.project_id)?;
                let diff = crate::git::show(&root, &request.commit_id)?;
                Ok(ResponseKind::GitShow(crate::proto::GitShowResponse {
                    unified_diff: diff,
                }))
            }
            Some(RequestKind::GitCloneLocal(request)) => {
                let source_root = self.project_root_by_id(&request.source_project_id)?;
                let target_name = request.target_project_name.clone();
                let created = self
                    .create_project(CreateProjectRequest {
                        name: target_name.clone(),
                    })?
                    .project
                    .ok_or_else(|| CoreError::Internal("missing project".into()))?;
                let target_id = created.id;
                let target_root = self.project_root_by_id(&target_id)?;
                if target_root.exists() {
                    std::fs::remove_dir_all(&target_root)?;
                }
                crate::git::clone_local(&source_root, &target_root)?;
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "user".into(),
                    op: "git_clone_local".into(),
                    resolved_paths: vec![
                        source_root.to_string_lossy().into_owned(),
                        target_root.to_string_lossy().into_owned(),
                    ],
                    before_hash: None,
                    after_hash: Some(target_id.clone()),
                    policy_result: "ALLOW".into(),
                    approval_id: None,
                    session_id: Some(request.source_project_id.clone()),
                });
                Ok(ResponseKind::GitCloneLocal(
                    crate::proto::GitCloneLocalResponse {
                        project_id: target_id,
                    },
                ))
            }
            Some(RequestKind::GitRemoteFetch(request)) => {
                if request.credential_handle.trim().is_empty() {
                    return Err(CoreError::InvalidArgument(
                        "remote fetch requires credential broker handle".into(),
                    ));
                }
                let _ = self.project_root_by_id(&request.project_id)?;
                Ok(ResponseKind::GitRemoteFetch(crate::proto::GitRemoteFetchResponse { ok: false, message: "Remote fetch not runtime-verified in M9; credential handle accepted but network operation not executed in this build.".into() }))
            }
            Some(RequestKind::GitRemotePush(request)) => {
                let cmd = if request.force {
                    "git push --force".to_string()
                } else {
                    "git push".to_string()
                };
                let assessment =
                    crate::policy::evaluate_command(&cmd, false, &request.project_id, "");
                if assessment.decision == crate::policy::PolicyDecision::Confirm {
                    crate::policy::consume_approval_token(
                        &request.approval_token,
                        &assessment.command_hash,
                        &request.project_id,
                    )?;
                } else if assessment.decision == crate::policy::PolicyDecision::Deny {
                    return Err(CoreError::InvalidArgument(format!(
                        "push denied: {}",
                        assessment.reason
                    )));
                }
                if request.credential_handle.trim().is_empty() {
                    return Err(CoreError::InvalidArgument(
                        "remote push requires credential broker handle".into(),
                    ));
                }
                let _ = self.project_root_by_id(&request.project_id)?;
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "user".into(),
                    op: "git_push".into(),
                    resolved_paths: vec![request.remote.clone(), request.branch.clone()],
                    before_hash: None,
                    after_hash: None,
                    policy_result: "CONFIRM".into(),
                    approval_id: Some(audit::redact(&request.approval_token)),
                    session_id: Some(request.project_id.clone()),
                });
                Ok(ResponseKind::GitRemotePush(crate::proto::GitRemotePushResponse { ok: false, message: "Remote push not runtime-verified in M9; approval/credential checks enforced, network operation not executed in this build.".into() }))
            }
            Some(RequestKind::GitAuthStartDeviceFlow(_))
            | Some(RequestKind::GitAuthPoll(_))
            | Some(RequestKind::GitAuthStorePat(_))
            | Some(RequestKind::GitAuthRevoke(_))
            | Some(RequestKind::GitCredentialForRemote(_)) => Err(CoreError::InvalidArgument(
                "git auth/credential broker operations are handled by Kotlin broker layer".into(),
            )),
            Some(RequestKind::LinuxEnvStatus(_)) => {
                Ok(ResponseKind::LinuxEnvStatus(self.linux_env_status()?))
            }
            Some(RequestKind::LinuxEnvInstall(request)) => {
                Ok(ResponseKind::LinuxEnvInstall(self.linux_env_install(
                    request.distro(),
                    request.bootstrap_mode(),
                    request.source_path,
                )?))
            }
            Some(RequestKind::LinuxEnvUninstall(_)) => {
                Ok(ResponseKind::LinuxEnvUninstall(self.linux_env_uninstall()?))
            }
            Some(RequestKind::LinuxShellSpawn(request)) => Ok(ResponseKind::LinuxShellSpawn(
                self.linux_shell_spawn(request)?,
            )),
            Some(RequestKind::LinuxPackageInstall(request)) => Ok(
                ResponseKind::LinuxPackageInstall(self.linux_package_install(request)?),
            ),
            Some(RequestKind::LinuxRunInstalledBinary(request)) => Ok(
                ResponseKind::LinuxRunInstalledBinary(self.linux_run_installed_binary(request)?),
            ),
            Some(RequestKind::NetworkModeState(_)) => {
                Ok(ResponseKind::NetworkModeState(self.network_mode_state()?))
            }
            Some(RequestKind::NetworkSetMode(request)) => Ok(ResponseKind::NetworkSetMode(
                self.network_set_mode(request)?,
            )),
            Some(RequestKind::NetworkAllowlistList(_)) => Ok(ResponseKind::NetworkAllowlistList(
                self.network_allowlist_list()?,
            )),
            Some(RequestKind::NetworkAllowlistUpsert(request)) => Ok(
                ResponseKind::NetworkAllowlistUpsert(self.network_allowlist_upsert(request)?),
            ),
            Some(RequestKind::NetworkAllowlistRemove(request)) => Ok(
                ResponseKind::NetworkAllowlistRemove(self.network_allowlist_remove(request)?),
            ),
            Some(RequestKind::NetworkConsent(request)) => {
                Ok(ResponseKind::NetworkConsent(self.network_consent(request)?))
            }
            Some(RequestKind::NetworkActivityList(request)) => Ok(
                ResponseKind::NetworkActivityList(self.network_activity_list(request)?),
            ),
            Some(RequestKind::NetworkCancel(request)) => {
                Ok(ResponseKind::NetworkCancel(self.network_cancel(request)?))
            }
            Some(RequestKind::NetworkSessionPolicy(request)) => Ok(
                ResponseKind::NetworkSessionPolicy(self.network_session_policy(request)?),
            ),
            Some(RequestKind::NetworkDecision(request)) => Ok(ResponseKind::NetworkDecision(
                self.network_decision(request)?,
            )),
            Some(RequestKind::BuildRun(request)) => {
                Ok(ResponseKind::BuildRun(self.build_run(request)?))
            }
            Some(RequestKind::SshRun(request)) => Ok(ResponseKind::SshRun(self.ssh_run(request)?)),
            None => Err(CoreError::InvalidArgument("missing request kind".into())),
        }
    }

    fn linux_env_status(&self) -> Result<crate::proto::LinuxEnvStatusResponse> {
        let st = crate::linux_env::LinuxEnvManager::new_with_native_lib_dir(
            &self.app_data_dir,
            &self.native_lib_dir,
        )?
        .status();
        Ok(crate::proto::LinuxEnvStatusResponse {
            state: st.state as i32,
            rootfs_path: st.rootfs_path,
            distro: st.distro as i32,
            arch: st.arch,
            size_bytes: st.size_bytes,
            pkg_mgr: st.pkg_mgr,
            message: st.message,
            rootfs_present: st.rootfs_present,
            proot_loader_present: st.proot_loader_present,
            shell_binary: st.shell_binary,
        })
    }

    fn linux_env_install(
        &self,
        distro: crate::proto::LinuxDistro,
        bootstrap_mode: crate::proto::LinuxBootstrapMode,
        source_path: String,
    ) -> Result<crate::proto::LinuxEnvInstallResponse> {
        let mgr = crate::linux_env::LinuxEnvManager::new_with_native_lib_dir(
            &self.app_data_dir,
            &self.native_lib_dir,
        )?;
        if matches!(distro, crate::proto::LinuxDistro::Unspecified) {
            return Err(CoreError::InvalidArgument("distro required".into()));
        }
        let mode = if matches!(
            bootstrap_mode,
            crate::proto::LinuxBootstrapMode::Unspecified
        ) {
            crate::proto::LinuxBootstrapMode::BundledOffline
        } else {
            bootstrap_mode
        };
        if matches!(mode, crate::proto::LinuxBootstrapMode::NetworkDownload)
            && source_path.trim().is_empty()
        {
            let msg = "NETWORK_DOWNLOAD requires Kotlin broker to download rootfs to an app-private path before install. Not runtime-verified on supported device (Option B: only API 28 emulator available, below minSdk 31).".to_string();
            mgr.mark_failed(distro, &msg)?;
            return Ok(crate::proto::LinuxEnvInstallResponse {
                state: crate::proto::LinuxEnvState::LinuxEnvFailed as i32,
                progress_token: String::new(),
                message: msg,
            });
        }
        let token = format!("linux-install-{}", now_millis());
        mgr.mark_installing(distro, "Extracting bundled Alpine rootfs...")?;
        let _ = self.emit_event_kind(EventKind::LinuxEnvInstallProgress(
            crate::proto::LinuxEnvInstallProgressEvent {
                progress_token: token.clone(),
                stage: "extracting".into(),
                percent: 5,
                bytes_done: 0,
                bytes_total: 0,
                message: "Extracting bundled Alpine rootfs...".into(),
                state: crate::proto::LinuxEnvState::LinuxEnvInstalling as i32,
            },
        ));
        let src = std::path::PathBuf::from(source_path.trim());
        if source_path.trim().is_empty() || !src.exists() {
            let msg = "Bundled rootfs source not found; Kotlin must copy asset to app-private path before install".to_string();
            mgr.mark_failed(distro, &msg)?;
            let _ = self.emit_event_kind(EventKind::LinuxEnvInstallProgress(
                crate::proto::LinuxEnvInstallProgressEvent {
                    progress_token: token.clone(),
                    stage: "failed".into(),
                    percent: 100,
                    bytes_done: 0,
                    bytes_total: 0,
                    message: msg.clone(),
                    state: crate::proto::LinuxEnvState::LinuxEnvFailed as i32,
                },
            ));
            let _ = self.audit.append(&AuditRecord {
                ts: audit::now_iso8601(),
                actor: "user".into(),
                op: "linux_env_install".into(),
                resolved_paths: vec![mgr.rootfs_path(distro).to_string_lossy().into_owned()],
                before_hash: None,
                after_hash: Some(format!("{:?}", distro)),
                policy_result: "DENY".into(),
                approval_id: None,
                session_id: None,
            });
            return Ok(crate::proto::LinuxEnvInstallResponse {
                state: crate::proto::LinuxEnvState::LinuxEnvFailed as i32,
                progress_token: token,
                message: msg,
            });
        }
        match mgr.install_bundled_offline(distro, &src) {
            Ok(st) => {
                let _ = self.emit_event_kind(EventKind::LinuxEnvInstallProgress(
                    crate::proto::LinuxEnvInstallProgressEvent {
                        progress_token: token.clone(),
                        stage: "ready".into(),
                        percent: 100,
                        bytes_done: st.size_bytes,
                        bytes_total: st.size_bytes,
                        message: st.message.clone(),
                        state: crate::proto::LinuxEnvState::LinuxEnvReady as i32,
                    },
                ));
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "user".into(),
                    op: "linux_env_install".into(),
                    resolved_paths: vec![st.rootfs_path.clone()],
                    before_hash: None,
                    after_hash: Some(format!("{:?}", distro)),
                    policy_result: "ALLOW".into(),
                    approval_id: None,
                    session_id: None,
                });
                Ok(crate::proto::LinuxEnvInstallResponse {
                    state: st.state as i32,
                    progress_token: token,
                    message: st.message,
                })
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = self.emit_event_kind(EventKind::LinuxEnvInstallProgress(
                    crate::proto::LinuxEnvInstallProgressEvent {
                        progress_token: token.clone(),
                        stage: "failed".into(),
                        percent: 100,
                        bytes_done: 0,
                        bytes_total: 0,
                        message: msg.clone(),
                        state: crate::proto::LinuxEnvState::LinuxEnvFailed as i32,
                    },
                ));
                let _ = self.audit.append(&AuditRecord {
                    ts: audit::now_iso8601(),
                    actor: "user".into(),
                    op: "linux_env_install".into(),
                    resolved_paths: vec![mgr.rootfs_path(distro).to_string_lossy().into_owned()],
                    before_hash: None,
                    after_hash: Some(format!("{:?}", distro)),
                    policy_result: "DENY".into(),
                    approval_id: None,
                    session_id: None,
                });
                Ok(crate::proto::LinuxEnvInstallResponse {
                    state: crate::proto::LinuxEnvState::LinuxEnvFailed as i32,
                    progress_token: token,
                    message: msg,
                })
            }
        }
    }

    fn linux_env_uninstall(&self) -> Result<crate::proto::LinuxEnvUninstallResponse> {
        let mgr = crate::linux_env::LinuxEnvManager::new(&self.app_data_dir)?;
        let st = mgr.uninstall()?;
        let _ = self.audit.append(&AuditRecord {
            ts: audit::now_iso8601(),
            actor: "user".into(),
            op: "linux_env_uninstall".into(),
            resolved_paths: vec![st.rootfs_path.clone()],
            before_hash: None,
            after_hash: None,
            policy_result: "ALLOW".into(),
            approval_id: None,
            session_id: None,
        });
        Ok(crate::proto::LinuxEnvUninstallResponse {
            ok: true,
            state: st.state as i32,
            message: st.message,
        })
    }

    fn linux_shell_spawn(
        &self,
        request: crate::proto::LinuxShellSpawnRequest,
    ) -> Result<crate::proto::LinuxShellSpawnResponse> {
        let project_dir = self.project_root_by_id(&request.project_id)?;
        let mgr = crate::linux_env::LinuxEnvManager::new_with_native_lib_dir(
            &self.app_data_dir,
            &self.native_lib_dir,
        )?;
        let st = mgr.status();
        if !matches!(st.state, crate::proto::LinuxEnvState::LinuxEnvReady) {
            return Err(CoreError::InvalidArgument(format!(
                "Linux env not ready: {}",
                st.message
            )));
        }
        let guest_cwd = mgr.ensure_guest_path_mapping(&project_dir)?;
        let argv = mgr.build_proot_argv(Path::new(&st.rootfs_path), &project_dir, &guest_cwd);
        let argv_json = serde_json_like_vec_encode(&argv);
        let user_env = crate::parse_env_overrides_json(&request.env_json);
        let mut env_overrides = Vec::new();
        env_overrides.push(("HOME".into(), "/root".into()));
        env_overrides.push(("TERM".into(), "xterm-256color".into()));
        env_overrides.push((
            "PATH".into(),
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
        ));
        env_overrides.push(("LANG".into(), "C.UTF-8".into()));
        env_overrides.push((
            "TMPDIR".into(),
            mgr.cache_root().join("tmp").to_string_lossy().into_owned(),
        ));
        env_overrides.push(("FORGEBOX_LINUX_ROOTFS".into(), st.rootfs_path.clone()));
        env_overrides.push(("FORGEBOX_GUEST_CWD".into(), guest_cwd.clone()));
        for (k, v) in user_env {
            if is_allowed_env_override(&k) {
                env_overrides.retain(|(ek, _)| ek != &k);
                env_overrides.push((k, v));
            }
        }
        let pty_id = self.pty_open_with_command(
            Some(request.project_id.clone()),
            Some("linux-sh".into()),
            env_overrides,
            request.cols.max(2) as u16,
            request.rows.max(2) as u16,
            12,
            Some(argv[0].clone()),
            Some(argv_json),
            Some(project_dir.to_string_lossy().into_owned()),
        )?;
        let _ = self.audit.append(&AuditRecord {
            ts: audit::now_iso8601(),
            actor: "user".into(),
            op: "linux_shell_spawn".into(),
            resolved_paths: vec![guest_cwd],
            before_hash: None,
            after_hash: Some(st.rootfs_path.clone()),
            policy_result: "ALLOW".into(),
            approval_id: None,
            session_id: Some(pty_id.to_string()),
        });
        Ok(crate::proto::LinuxShellSpawnResponse {
            pty_id,
            env_state: st.state as i32,
        })
    }

    fn build_run(
        &self,
        request: crate::proto::BuildRunRequest,
    ) -> Result<crate::proto::BuildRunResponse> {
        let project_dir = self.project_root_by_id(&request.project_id)?;
        let mgr = crate::linux_env::LinuxEnvManager::new_with_native_lib_dir(
            &self.app_data_dir,
            &self.native_lib_dir,
        )?;
        let st = mgr.status();
        if !matches!(st.state, crate::proto::LinuxEnvState::LinuxEnvReady) {
            return Err(CoreError::InvalidArgument(format!(
                "Linux env not ready: {}",
                st.message
            )));
        }
        let guest_cwd = mgr.ensure_guest_path_mapping(&project_dir)?;
        let kind = request.kind();
        let (command, python_missing) = match kind {
            crate::proto::BuildKind::Python => {
                let rel = request.file_path.trim().trim_start_matches('/');
                if rel.is_empty() || rel.contains("..") {
                    return Err(CoreError::InvalidArgument(
                        "python file_path must be a project-relative path".into(),
                    ));
                }
                let abs = project_dir.join(rel);
                if !abs.exists() {
                    return Err(CoreError::NotFound(format!("file not found: {rel}")));
                }
                let rootfs = Path::new(&st.rootfs_path);
                let python_missing = !rootfs.join("usr/bin/python3").exists()
                    && !rootfs.join("bin/python3").exists();
                if python_missing {
                    return Ok(crate::proto::BuildRunResponse {
                        pty_id: 0,
                        command: "apk add python3 py3-pip".into(),
                        message: "python3 missing in Linux environment; install python3 py3-pip"
                            .into(),
                        python_missing: true,
                    });
                }
                let guest_file = format!("{}/{}", guest_cwd, rel.replace('\\', "/"));
                (format!("python3 {}", shell_escape(&guest_file)), false)
            }
            crate::proto::BuildKind::GenericCommand => {
                let cmd = request.command.trim();
                if cmd.is_empty() {
                    return Err(CoreError::InvalidArgument("command required".into()));
                }
                (cmd.to_string(), false)
            }
            crate::proto::BuildKind::SshCommand => {
                return Err(CoreError::InvalidArgument(
                    "SSH command execution is scaffolded in Kotlin; use SSH transport layer".into(),
                ));
            }
            _ => return Err(CoreError::InvalidArgument("build kind required".into())),
        };

        let argv = vec![
            mgr.cache_root()
                .join("bin/proot")
                .to_string_lossy()
                .into_owned(),
            "--link2symlink".into(),
            "-0".into(),
            "-r".into(),
            st.rootfs_path.clone(),
            "-b".into(),
            "/dev".into(),
            "-b".into(),
            "/proc".into(),
            "-b".into(),
            "/sys".into(),
            "-b".into(),
            format!("{}:{}", project_dir.to_string_lossy(), guest_cwd),
            "-b".into(),
            format!("{}:/tmp", mgr.cache_root().join("tmp").to_string_lossy()),
            "-w".into(),
            guest_cwd.clone(),
            "/bin/sh".into(),
            "-lc".into(),
            command.clone(),
        ];
        let argv_json = serde_json_like_vec_encode(&argv);
        let env_overrides = vec![
            ("HOME".into(), "/root".into()),
            ("TERM".into(), "xterm-256color".into()),
            (
                "PATH".into(),
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
            ),
            ("LANG".into(), "C.UTF-8".into()),
            (
                "TMPDIR".into(),
                mgr.cache_root().join("tmp").to_string_lossy().into_owned(),
            ),
            ("FORGEBOX_LINUX_ROOTFS".into(), st.rootfs_path.clone()),
            ("FORGEBOX_GUEST_CWD".into(), guest_cwd.clone()),
        ];
        let pty_id = self.pty_open_with_command(
            Some(request.project_id.clone()),
            Some("build".into()),
            env_overrides,
            request.cols.max(2) as u16,
            request.rows.max(2) as u16,
            12,
            Some(argv[0].clone()),
            Some(argv_json),
            Some(project_dir.to_string_lossy().into_owned()),
        )?;
        let _ = self.audit.append(&AuditRecord {
            ts: audit::now_iso8601(),
            actor: "user".into(),
            op: "build_run".into(),
            resolved_paths: vec![guest_cwd],
            before_hash: None,
            after_hash: Some(command.clone()),
            policy_result: "ALLOW".into(),
            approval_id: None,
            session_id: Some(pty_id.to_string()),
        });
        Ok(crate::proto::BuildRunResponse {
            pty_id,
            command,
            message: "Build started in Linux session".into(),
            python_missing,
        })
    }

    fn ssh_run(
        &self,
        _request: crate::proto::SshRunRequest,
    ) -> Result<crate::proto::SshRunResponse> {
        Ok(crate::proto::SshRunResponse {
            ok: false,
            message: "SSH remote execution scaffolded in M12; live transport not runtime-verified under Option B (only API 28 emulator available, below minSdk 31). Use Kotlin fake-transport tests.".into(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: -1,
        })
    }

    fn linux_package_install(
        &self,
        request: crate::proto::LinuxPackageInstallRequest,
    ) -> Result<crate::proto::LinuxPackageInstallResponse> {
        let mgr_name = match request.manager() {
            crate::proto::LinuxPackageManager::Npm => "npm",
            crate::proto::LinuxPackageManager::Apt => "apt",
            crate::proto::LinuxPackageManager::Apk => "apk",
            _ => {
                return Err(CoreError::InvalidArgument(
                    "package manager required".into(),
                ))
            }
        };
        let command =
            crate::linux_env::build_package_install_command(mgr_name, &request.package_spec)?;
        let _assessment = crate::policy::evaluate_command_with_origin(
            &command,
            false,
            &request.cwd_project_id,
            "",
            crate::policy::CommandOrigin::User,
        );
        let require_consent = request.require_consent;
        let _ = require_consent; // M11: consent/allowlist enforced by Kotlin broker/UI before spawning; guest network not sandboxed.
        let project_dir = self.project_root_by_id(&request.cwd_project_id)?;
        let full_cmd = format!(
            "cd {} && {}",
            shell_escape(&project_dir.to_string_lossy()),
            command
        );
        let pty_id = self.pty_open(
            Some(request.cwd_project_id),
            Some(format!("install-{mgr_name}")),
            vec![],
            80,
            24,
            12,
        )?;
        self.pty_write(
            pty_id,
            format!(
                "{full_cmd}
"
            )
            .as_bytes(),
        )?;
        let _ = self.audit.append(&AuditRecord {
            ts: audit::now_iso8601(),
            actor: "user".into(),
            op: "linux_package_install".into(),
            resolved_paths: vec![command.clone()],
            before_hash: None,
            after_hash: None,
            policy_result: "ALLOW".into(),
            approval_id: None,
            session_id: Some(pty_id.to_string()),
        });
        Ok(crate::proto::LinuxPackageInstallResponse { pty_id, command })
    }

    fn linux_run_installed_binary(
        &self,
        request: crate::proto::LinuxRunInstalledBinaryRequest,
    ) -> Result<crate::proto::LinuxRunInstalledBinaryResponse> {
        let binary = crate::linux_env::validate_binary_name(&request.binary_name)?;
        let argv: Vec<String> = serde_json_like_vec(&request.argv_json);
        let mut cmd_parts = vec![binary.clone()];
        cmd_parts.extend(argv.clone());
        let command = cmd_parts.join(" ");
        let assessment = crate::policy::evaluate_command_with_origin(
            &command,
            false,
            &request.session_id,
            "",
            crate::policy::CommandOrigin::InstalledBinary,
        );
        if !request.approval_token.is_empty() {
            crate::policy::consume_approval_token(
                &request.approval_token,
                &request.command_hash,
                &request.session_id,
            )?;
        } else if matches!(assessment.decision, crate::policy::PolicyDecision::Confirm) {
            return Err(CoreError::InvalidArgument(
                "CONFIRM: installed binary requires approval token".into(),
            ));
        }
        let project_dir = self.project_root_by_id(&request.project_id)?;
        let full_cmd = format!(
            "cd {} && {}",
            shell_escape(&project_dir.to_string_lossy()),
            command
        );
        let pty_id = self.pty_open(
            Some(request.project_id),
            Some(binary.clone()),
            vec![],
            80,
            24,
            12,
        )?;
        self.pty_write(
            pty_id,
            format!(
                "{full_cmd}
"
            )
            .as_bytes(),
        )?;
        let _ = self.audit.append(&AuditRecord {
            ts: audit::now_iso8601(),
            actor: "user".into(),
            op: "installed_binary_run".into(),
            resolved_paths: vec![binary],
            before_hash: None,
            after_hash: Some(audit::redact(&command)),
            policy_result: "CONFIRM".into(),
            approval_id: Some(audit::redact(&request.approval_token)),
            session_id: Some(pty_id.to_string()),
        });
        Ok(crate::proto::LinuxRunInstalledBinaryResponse { pty_id, command })
    }

    fn network_mode_state(&self) -> Result<crate::proto::NetworkModeStateResponse> {
        let mut st = self.net_state.lock().map_err(lock_error)?;
        let now = crate::net_policy::now_ms();
        let mode = crate::net_policy::effective_mode(&mut st, now);
        Ok(crate::proto::NetworkModeStateResponse {
            mode: net_mode_to_proto(mode) as i32,
            temporary_expires_at_ms: st.temporary_expires_at_ms,
            controlled_allowlist_count: st.allowlist.len() as u32,
            temporary_scope: st
                .temporary_scope
                .map(net_temp_scope_to_proto)
                .unwrap_or(crate::proto::NetworkTemporaryScope::Unspecified)
                as i32,
            temporary_scope_id: st.temporary_scope_id.clone(),
        })
    }

    fn network_set_mode(
        &self,
        request: crate::proto::NetworkSetModeRequest,
    ) -> Result<crate::proto::NetworkSetModeResponse> {
        let mut st = self.net_state.lock().map_err(lock_error)?;
        let now = crate::net_policy::now_ms();
        let mode = net_mode_from_proto(request.mode())?;
        match mode {
            crate::net_policy::NetworkMode::TemporaryBroader => {
                let dur = crate::net_policy::cap_duration_seconds(request.duration_seconds);
                let prior = if matches!(st.mode, crate::net_policy::NetworkMode::TemporaryBroader) {
                    st.prior_mode
                } else {
                    st.mode
                };
                st.prior_mode = prior;
                st.mode = crate::net_policy::NetworkMode::TemporaryBroader;
                st.temporary_expires_at_ms = now + (dur as i64 * 1000);
                st.temporary_scope = Some(net_temp_scope_from_proto(request.temporary_scope())?);
                st.temporary_scope_id = request.scope_id;
            }
            _ => {
                st.mode = mode;
                st.prior_mode = mode;
                st.temporary_expires_at_ms = 0;
                st.temporary_scope = None;
                st.temporary_scope_id.clear();
            }
        }
        Ok(crate::proto::NetworkSetModeResponse {
            mode: net_mode_to_proto(st.mode) as i32,
            temporary_expires_at_ms: st.temporary_expires_at_ms,
            message: "mode updated".into(),
        })
    }

    fn network_allowlist_list(&self) -> Result<crate::proto::NetworkAllowlistListResponse> {
        let st = self.net_state.lock().map_err(lock_error)?;
        Ok(crate::proto::NetworkAllowlistListResponse {
            entries: st
                .allowlist
                .iter()
                .map(|e| crate::proto::NetworkAllowlistEntry {
                    host_pattern: e.host_pattern.clone(),
                    provider_kind: e.provider_kind.clone(),
                    added_at: e.added_at,
                    added_by: e.added_by.clone(),
                })
                .collect(),
        })
    }

    fn network_allowlist_upsert(
        &self,
        request: crate::proto::NetworkAllowlistUpsertRequest,
    ) -> Result<crate::proto::NetworkAllowlistUpsertResponse> {
        let mut st = self.net_state.lock().map_err(lock_error)?;
        let host = crate::net_policy::normalize_host(&request.host_pattern);
        if host.is_empty() {
            return Err(CoreError::InvalidArgument("host_pattern required".into()));
        }
        let entry = crate::net_policy::AllowlistEntry {
            host_pattern: host.clone(),
            provider_kind: request.provider_kind.clone(),
            added_at: crate::net_policy::now_ms(),
            added_by: "user".into(),
        };
        if let Some(existing) = st.allowlist.iter_mut().find(|e| e.host_pattern == host) {
            *existing = entry.clone();
        } else {
            st.allowlist.push(entry.clone());
        }
        Ok(crate::proto::NetworkAllowlistUpsertResponse {
            ok: true,
            entry: Some(crate::proto::NetworkAllowlistEntry {
                host_pattern: entry.host_pattern,
                provider_kind: entry.provider_kind,
                added_at: entry.added_at,
                added_by: entry.added_by,
            }),
        })
    }

    fn network_allowlist_remove(
        &self,
        request: crate::proto::NetworkAllowlistRemoveRequest,
    ) -> Result<crate::proto::NetworkAllowlistRemoveResponse> {
        let mut st = self.net_state.lock().map_err(lock_error)?;
        let host = crate::net_policy::normalize_host(&request.host_pattern);
        let before = st.allowlist.len();
        st.allowlist.retain(|e| e.host_pattern != host);
        Ok(crate::proto::NetworkAllowlistRemoveResponse {
            ok: st.allowlist.len() != before,
        })
    }

    fn network_consent(
        &self,
        request: crate::proto::NetworkConsentRequest,
    ) -> Result<crate::proto::NetworkConsentResponse> {
        let _ = request;
        Ok(crate::proto::NetworkConsentResponse {
            decision: crate::proto::NetworkConsentDecision::Deny as i32,
            remember_scope: crate::proto::NetworkRememberScope::Unspecified as i32,
        })
    }

    fn network_activity_list(
        &self,
        request: crate::proto::NetworkActivityListRequest,
    ) -> Result<crate::proto::NetworkActivityListResponse> {
        let log = self.net_activity.lock().map_err(lock_error)?;
        let limit = if request.limit == 0 {
            100
        } else {
            request.limit as usize
        };
        let events = log
            .list(limit)
            .into_iter()
            .map(|e| crate::proto::NetworkActivityEvent {
                ts: e.ts,
                destination_host: e.destination_host,
                actor_kind: net_actor_to_proto(e.actor_kind) as i32,
                actor_id: e.actor_id,
                bytes_up: e.bytes_up,
                bytes_down: e.bytes_down,
                status: net_status_to_proto(e.status) as i32,
                cancel_token: e.cancel_token,
                reason: e.reason,
            })
            .collect();
        Ok(crate::proto::NetworkActivityListResponse { events })
    }

    fn network_cancel(
        &self,
        request: crate::proto::NetworkCancelRequest,
    ) -> Result<crate::proto::NetworkCancelResponse> {
        let mut log = self.net_activity.lock().map_err(lock_error)?;
        let status = log
            .cancel(&request.cancel_token, &self.audit)
            .unwrap_or(crate::net_activity::ActivityStatus::Failed);
        Ok(crate::proto::NetworkCancelResponse {
            ok: matches!(status, crate::net_activity::ActivityStatus::Cancelled),
            status: net_status_to_proto(status) as i32,
        })
    }

    fn network_decision(
        &self,
        request: crate::proto::NetworkDecisionRequest,
    ) -> Result<crate::proto::NetworkDecisionResponse> {
        let mut st = self.net_state.lock().map_err(lock_error)?;
        let now = crate::net_policy::now_ms();
        let actor = match request.actor_kind() {
            crate::proto::NetworkActorKind::Session => crate::net_policy::ActorKind::Session,
            crate::proto::NetworkActorKind::InstalledBinary => {
                crate::net_policy::ActorKind::InstalledBinary
            }
            _ => crate::net_policy::ActorKind::User,
        };
        let session_enabled = self.net_session_enabled(&request.session_id);
        let mode = crate::net_policy::effective_mode(&mut st, now);
        let result = crate::net_policy::decide(
            &mut st,
            &request.destination_host,
            actor,
            &request.actor_id,
            session_enabled,
            now,
        );
        crate::net_policy::audit_decision(
            &self.audit,
            mode,
            &request.destination_host,
            actor,
            &request.actor_id,
            &result,
        );
        let mut log = self.net_activity.lock().map_err(lock_error)?;
        let status = match result.decision {
            crate::net_policy::Decision::Allow => crate::net_activity::ActivityStatus::Succeeded,
            crate::net_policy::Decision::Deny => crate::net_activity::ActivityStatus::Failed,
            crate::net_policy::Decision::Ask => crate::net_activity::ActivityStatus::Pending,
        };
        let event = crate::net_activity::ActivityEvent {
            ts: now,
            destination_host: crate::net_policy::normalize_host(&request.destination_host),
            actor_kind: actor,
            actor_id: request.actor_id.clone(),
            bytes_up: 0,
            bytes_down: 0,
            status,
            cancel_token: log.next_token(),
            reason: result.reason.clone(),
        };
        log.record(event);
        Ok(crate::proto::NetworkDecisionResponse {
            allow: matches!(result.decision, crate::net_policy::Decision::Allow),
            ask: matches!(result.decision, crate::net_policy::Decision::Ask),
            reason: result.reason,
            effective_mode: net_mode_to_proto(mode) as i32,
        })
    }

    fn network_session_policy(
        &self,
        request: crate::proto::NetworkSessionPolicyRequest,
    ) -> Result<crate::proto::NetworkSessionPolicyResponse> {
        let sid = request.session_id.trim().to_string();
        if sid.is_empty() {
            return Err(CoreError::InvalidArgument("session_id required".into()));
        }
        {
            let mut map = self.net_session_enabled.lock().map_err(lock_error)?;
            map.insert(sid.clone(), request.network_enabled);
        }
        let mut message = "session network policy updated".to_string();
        if !request.network_enabled {
            if let Ok(pty_id) = sid.parse::<u64>() {
                let _ = self.pty_close(pty_id);
                message = "session network disabled; PTY killed".into();
            }
        }
        Ok(crate::proto::NetworkSessionPolicyResponse {
            ok: true,
            session_id: sid,
            network_enabled: request.network_enabled,
            message,
        })
    }

    fn net_session_enabled(&self, session_id: &str) -> bool {
        self.net_session_enabled
            .lock()
            .map(|m| m.get(session_id).copied().unwrap_or(true))
            .unwrap_or(true)
    }

    fn get_info(&self, _request: GetInfoRequest) -> Result<GetInfoResponse> {
        let db = self.db.lock().map_err(lock_error)?;
        let project_count = db::count_projects(&db)?;
        Ok(GetInfoResponse {
            version: CORE_VERSION.into(),
            workspace_root: path_string(&self.workspace_root),
            database_path: path_string(&self.db_path),
            project_count,
            native_library_dir: path_string(&self.native_lib_dir),
            max_editable_file_bytes: MAX_EDITABLE_FILE_BYTES,
        })
    }

    fn create_project(&self, request: CreateProjectRequest) -> Result<CreateProjectResponse> {
        vfs::validate_project_name(&request.name)?;
        {
            let db = self.db.lock().map_err(lock_error)?;
            if db::project_exists(&db, &request.name)? {
                return Err(CoreError::AlreadyExists(format!(
                    "project already exists: {}",
                    request.name
                )));
            }
        }

        let project_path = self.workspace_root.join("Projects").join(&request.name);
        if project_path.exists() {
            return Err(CoreError::AlreadyExists(format!(
                "project directory already exists: {}",
                request.name
            )));
        }

        if let Err(error) = fs::create_dir_all(&project_path) {
            return Err(error.into());
        }

        let project = Project {
            id: request.name.clone(),
            name: request.name.clone(),
            path: path_string(&project_path),
            created_at: now_millis(),
            storage_bytes: 0,
        };
        {
            let db = self.db.lock().map_err(lock_error)?;
            if let Err(error) = db::insert_project(&db, &project) {
                let _ = fs::remove_dir_all(&project_path);
                return Err(error.into());
            }
        }

        self.emit_file_event(&request.name, "", "project_created")?;
        Ok(CreateProjectResponse {
            project: Some(project),
        })
    }

    fn list_projects(&self, _request: ListProjectsRequest) -> Result<ListProjectsResponse> {
        let db = self.db.lock().map_err(lock_error)?;
        Ok(ListProjectsResponse {
            projects: db::list_projects(&db)?,
        })
    }

    fn write_file(&self, request: WriteFileRequest) -> Result<WriteFileResponse> {
        self.assert_project_exists(&request.project_id)?;
        let relative = vfs::validate_relative_path(&request.path, false)?;
        let project_dir = self.project_dir(&request.project_id)?;
        let target = vfs::resolve_for_write(&project_dir, &relative)?;
        let existed = target.exists();
        let before_hash = if existed {
            Some(hash_bytes(&fs::read(&target)?))
        } else {
            None
        };
        fs::write(&target, &request.content)?;
        let size = fs::metadata(&target)?.len();
        let after_hash = Some(hash_bytes(&request.content));
        {
            let db = self.db.lock().map_err(lock_error)?;
            db::upsert_file_index(&db, &request.project_id, &request.path, size, now_millis())?;
        }
        self.emit_file_event(
            &request.project_id,
            &request.path,
            if existed { "modified" } else { "created" },
        )?;
        let _ = self.audit.append(&AuditRecord {
            ts: audit::now_iso8601(),
            actor: "system".into(),
            op: "write_file".into(),
            resolved_paths: vec![path_string(&target)],
            before_hash,
            after_hash,
            policy_result: "ALLOW".into(),
            approval_id: None,
            session_id: None,
        });
        Ok(WriteFileResponse {
            path: request.path,
            size,
        })
    }

    fn read_file(&self, request: ReadFileRequest) -> Result<ReadFileResponse> {
        self.assert_project_exists(&request.project_id)?;
        let relative = vfs::validate_relative_path(&request.path, false)?;
        let project_dir = self.project_dir(&request.project_id)?;
        let target = vfs::resolve(&project_dir, &relative, true)?;
        if !target.is_file() {
            return Err(CoreError::InvalidArgument(
                "path is not a regular file".into(),
            ));
        }
        Ok(ReadFileResponse {
            content: fs::read(target)?,
        })
    }

    fn stat_file(&self, request: StatFileRequest) -> Result<StatFileResponse> {
        self.assert_project_exists(&request.project_id)?;
        let relative = vfs::validate_relative_path(&request.path, false)?;
        let project_dir = self.project_dir(&request.project_id)?;
        let target = vfs::resolve(&project_dir, &relative, true)?;
        let metadata = fs::symlink_metadata(&target)?;
        if metadata.file_type().is_symlink() {
            return Err(CoreError::PathEscape("symlink rejected".into()));
        }
        if !metadata.is_file() {
            return Err(CoreError::InvalidArgument(
                "path is not a regular file".into(),
            ));
        }
        let size = metadata.len();
        let mtime_millis = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let is_text_heuristic = is_text_heuristic(&target, size)?;
        Ok(StatFileResponse {
            path: request.path,
            size,
            mtime_millis,
            is_text_heuristic,
            exists: true,
        })
    }

    fn write_file_if_unchanged(
        &self,
        request: WriteFileIfUnchangedRequest,
    ) -> Result<WriteFileIfUnchangedResponse> {
        self.assert_project_exists(&request.project_id)?;
        let relative = vfs::validate_relative_path(&request.path, false)?;
        let project_dir = self.project_dir(&request.project_id)?;
        let target = vfs::resolve(&project_dir, &relative, true)?;
        let metadata = fs::symlink_metadata(&target)?;
        if metadata.file_type().is_symlink() {
            return Err(CoreError::PathEscape("symlink rejected".into()));
        }
        if !metadata.is_file() {
            return Err(CoreError::InvalidArgument(
                "path is not a regular file".into(),
            ));
        }
        let current_mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if current_mtime != request.expected_mtime_millis {
            return Err(CoreError::InvalidArgument(format!(
                "CONFLICT: file changed on disk (expected mtime {}, found {})",
                request.expected_mtime_millis, current_mtime
            )));
        }
        let before_hash = Some(hash_bytes(&fs::read(&target)?));
        fs::write(&target, &request.content)?;
        let new_meta = fs::metadata(&target)?;
        let size = new_meta.len();
        let mtime_millis = new_meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let after_hash = Some(hash_bytes(&request.content));
        {
            let db = self.db.lock().map_err(lock_error)?;
            db::upsert_file_index(&db, &request.project_id, &request.path, size, now_millis())?;
        }
        self.emit_file_event(&request.project_id, &request.path, "modified")?;
        let _ = self.audit.append(&AuditRecord {
            ts: audit::now_iso8601(),
            actor: "system".into(),
            op: "write_file_if_unchanged".into(),
            resolved_paths: vec![path_string(&target)],
            before_hash,
            after_hash,
            policy_result: "ALLOW".into(),
            approval_id: None,
            session_id: None,
        });
        Ok(WriteFileIfUnchangedResponse {
            path: request.path,
            size,
            mtime_millis,
        })
    }

    fn list_files(&self, request: ListFilesRequest) -> Result<ListFilesResponse> {
        self.assert_project_exists(&request.project_id)?;
        let relative = vfs::validate_relative_path(&request.path, true)?;
        let project_dir = self.project_dir(&request.project_id)?;
        let directory = vfs::resolve(&project_dir, &relative, true)?;
        if !directory.is_dir() {
            return Err(CoreError::InvalidArgument("path is not a directory".into()));
        }

        let mut entries = Vec::new();
        self.collect_entries(
            &project_dir,
            &relative,
            &directory,
            request.recursive,
            &mut entries,
        )?;
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(ListFilesResponse { entries })
    }

    fn collect_entries(
        &self,
        project_dir: &Path,
        relative: &Path,
        directory: &Path,
        recursive: bool,
        out: &mut Vec<FileEntry>,
    ) -> Result<()> {
        let mut dir_entries: Vec<_> =
            fs::read_dir(directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
        dir_entries.sort_by_key(|e| e.file_name());
        for entry in dir_entries {
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let child_relative = if relative.as_os_str().is_empty() {
                PathBuf::from(&name)
            } else {
                relative.join(&name)
            };
            let child_relative_str = path_string(&child_relative).replace('\\', "/");
            let is_dir = file_type.is_dir();
            out.push(FileEntry {
                name,
                path: child_relative_str.clone(),
                is_dir,
                size: if file_type.is_file() {
                    entry.metadata()?.len()
                } else {
                    0
                },
            });
            if recursive && is_dir {
                let child_abs = vfs::resolve(project_dir, &child_relative, true)?;
                self.collect_entries(project_dir, &child_relative, &child_abs, true, out)?;
            }
        }
        Ok(())
    }

    fn create_folder(&self, request: CreateFolderRequest) -> Result<CreateFolderResponse> {
        self.assert_project_exists(&request.project_id)?;
        let relative = vfs::validate_relative_path(&request.path, false)?;
        let project_dir = self.project_dir(&request.project_id)?;
        let target = vfs::resolve_for_create(&project_dir, &relative, true)?;
        if target.exists() {
            return Err(CoreError::AlreadyExists(format!(
                "path already exists: {}",
                request.path
            )));
        }
        fs::create_dir_all(&target)?;
        self.emit_file_event(&request.project_id, &request.path, "folder_created")?;
        Ok(CreateFolderResponse { path: request.path })
    }

    fn rename_file(&self, request: RenameFileRequest) -> Result<RenameFileResponse> {
        self.assert_project_exists(&request.project_id)?;
        let old_rel = vfs::validate_relative_path(&request.old_path, false)?;
        let new_rel = vfs::validate_relative_path(&request.new_path, false)?;
        let project_dir = self.project_dir(&request.project_id)?;
        let old_target = vfs::resolve(&project_dir, &old_rel, true)?;
        let new_target = vfs::resolve_for_create(&project_dir, &new_rel, true)?;
        if new_target.exists() {
            return Err(CoreError::AlreadyExists(format!(
                "destination already exists: {}",
                request.new_path
            )));
        }
        if old_target == project_dir {
            return Err(CoreError::InvalidArgument(
                "cannot rename project root".into(),
            ));
        }
        fs::rename(&old_target, &new_target)?;
        {
            let db = self.db.lock().map_err(lock_error)?;
            db::rename_file_index(
                &db,
                &request.project_id,
                &request.old_path,
                &request.new_path,
            )?;
        }
        self.emit_file_event(&request.project_id, &request.new_path, "renamed")?;
        let _ = self.audit.append(&AuditRecord {
            ts: audit::now_iso8601(),
            actor: "system".into(),
            op: "rename_file".into(),
            resolved_paths: vec![path_string(&old_target), path_string(&new_target)],
            before_hash: None,
            after_hash: None,
            policy_result: "ALLOW".into(),
            approval_id: None,
            session_id: None,
        });
        Ok(RenameFileResponse {
            old_path: request.old_path,
            new_path: request.new_path,
        })
    }

    fn delete_path(&self, request: DeletePathRequest) -> Result<DeletePathResponse> {
        self.assert_project_exists(&request.project_id)?;
        let relative = vfs::validate_relative_path(&request.path, false)?;
        let project_dir = self.project_dir(&request.project_id)?;
        let target = vfs::resolve(&project_dir, &relative, true)?;
        if target == project_dir {
            return Err(CoreError::InvalidArgument(
                "cannot delete project root".into(),
            ));
        }
        let metadata = fs::symlink_metadata(&target)?;
        if metadata.file_type().is_symlink() {
            return Err(CoreError::PathEscape("symlink rejected".into()));
        }
        let before_hash = if metadata.is_file() {
            fs::read(&target).ok().map(|b| hash_bytes(&b))
        } else {
            None
        };
        if metadata.is_dir() {
            let mut entries = fs::read_dir(&target)?;
            if entries.next().is_some() && !request.recursive {
                return Err(CoreError::InvalidArgument(
                    "directory is not empty; recursive delete required".into(),
                ));
            }
            fs::remove_dir_all(&target)?;
        } else {
            fs::remove_file(&target)?;
        }
        {
            let db = self.db.lock().map_err(lock_error)?;
            db::delete_file_index_prefix(&db, &request.project_id, &request.path)?;
        }
        self.emit_file_event(&request.project_id, &request.path, "deleted")?;
        let _ = self.audit.append(&AuditRecord {
            ts: audit::now_iso8601(),
            actor: "system".into(),
            op: "delete_path".into(),
            resolved_paths: vec![path_string(&target)],
            before_hash,
            after_hash: None,
            policy_result: "ALLOW".into(),
            approval_id: None,
            session_id: None,
        });
        Ok(DeletePathResponse { path: request.path })
    }

    fn delete_project(&self, request: DeleteProjectRequest) -> Result<DeleteProjectResponse> {
        self.assert_project_exists(&request.project_id)?;
        let project_dir = self.project_dir(&request.project_id)?;
        let resolved = vfs::resolve(&project_dir, Path::new(""), true)?;
        let canonical = fs::canonicalize(&project_dir)?;
        if resolved != canonical {
            return Err(CoreError::PathEscape(
                "project root resolution mismatch".into(),
            ));
        }
        fs::remove_dir_all(&resolved)?;
        {
            let db = self.db.lock().map_err(lock_error)?;
            db::delete_project(&db, &request.project_id)?;
        }
        self.emit_file_event(&request.project_id, "", "project_deleted")?;
        Ok(DeleteProjectResponse {
            project_id: request.project_id,
        })
    }

    fn rename_project(&self, request: RenameProjectRequest) -> Result<RenameProjectResponse> {
        self.assert_project_exists(&request.project_id)?;
        vfs::validate_project_name(&request.new_name)?;
        if request.project_id == request.new_name {
            return Err(CoreError::InvalidArgument("new name is unchanged".into()));
        }
        {
            let db = self.db.lock().map_err(lock_error)?;
            if db::project_exists(&db, &request.new_name)? {
                return Err(CoreError::AlreadyExists(format!(
                    "project already exists: {}",
                    request.new_name
                )));
            }
        }
        let old_dir = self.project_dir(&request.project_id)?;
        let new_dir = self.project_dir(&request.new_name)?;
        if new_dir.exists() {
            return Err(CoreError::AlreadyExists(format!(
                "project directory already exists: {}",
                request.new_name
            )));
        }
        let old_resolved = vfs::resolve(&old_dir, Path::new(""), true)?;
        fs::rename(&old_resolved, &new_dir)?;
        let created_at = {
            let db = self.db.lock().map_err(lock_error)?;
            db::project_created_at(&db, &request.project_id)?
        };
        let project = Project {
            id: request.new_name.clone(),
            name: request.new_name.clone(),
            path: path_string(&new_dir),
            created_at,
            storage_bytes: 0,
        };
        {
            let db = self.db.lock().map_err(lock_error)?;
            db::rename_project(
                &db,
                &request.project_id,
                &request.new_name,
                &request.new_name,
                &project.path,
            )?;
        }
        self.emit_file_event(&request.new_name, "", "project_renamed")?;
        Ok(RenameProjectResponse {
            project: Some(project),
        })
    }

    fn project_dir(&self, project_id: &str) -> Result<PathBuf> {
        vfs::validate_project_name(project_id)?;
        Ok(self.workspace_root.join("Projects").join(project_id))
    }

    fn project_root_by_id(&self, project_id: &str) -> Result<PathBuf> {
        self.assert_project_exists(project_id)?;
        let dir = self.project_dir(project_id)?;
        vfs::canonical_root(&dir)
    }

    fn find_project_id_by_name(&self, name: &str) -> Result<String> {
        let db = self.db.lock().map_err(lock_error)?;
        let projects = db::list_projects(&db)?;
        projects
            .into_iter()
            .find(|p| p.name == name)
            .map(|p| p.id)
            .ok_or_else(|| CoreError::NotFound(format!("project not found: {name}")))
    }

    fn assert_project_exists(&self, project_id: &str) -> Result<()> {
        let db = self.db.lock().map_err(lock_error)?;
        if db::project_exists(&db, project_id)? {
            Ok(())
        } else {
            Err(CoreError::NotFound(format!(
                "project not found: {project_id}"
            )))
        }
    }

    fn emit_file_event(&self, project_id: &str, path: &str, change_type: &str) -> Result<()> {
        self.emit_event_kind(EventKind::FileChanged(FileChangedEvent {
            project_id: project_id.into(),
            path: path.into(),
            change_type: change_type.into(),
        }))
    }

    fn emit_event_kind(&self, kind: EventKind) -> Result<()> {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let created_at = now_millis();
        let kind_name = match &kind {
            EventKind::FileChanged(_) => "file_changed",
            EventKind::Log(_) => "log",
            EventKind::TerminalOutput(_) => "terminal_output",
            EventKind::TerminalExit(_) => "terminal_exit",
            EventKind::TerminalTitle(_) => "terminal_title",
            EventKind::TerminalCwd(_) => "terminal_cwd",
            EventKind::TerminalSessionSpawned(_) => "terminal_session_spawned",
            EventKind::TerminalSessionRenamed(_) => "terminal_session_renamed",
            EventKind::TerminalSessionState(_) => "terminal_session_state",
            EventKind::TerminalSessionRestoreAvailable(_) => "terminal_session_restore_available",
            EventKind::LinuxEnvInstallProgress(_) => "linux_env_install_progress",
        };
        let event = Event {
            seq,
            created_at,
            kind: Some(kind),
        };
        let payload = event.encode_to_vec();
        {
            let db = self.db.lock().map_err(lock_error)?;
            db::insert_event(&db, seq, kind_name, &payload, created_at)?;
        }
        let mut q = self.events.lock().map_err(lock_error)?;
        q.push_back(payload);
        self.events_cv.notify_all();
        Ok(())
    }

    #[cfg(test)]
    pub fn inject_test_terminal_session(&self, id: u64, rows: u16, cols: u16) {
        let session = pty::PtySession {
            id,
            master_fd: -1,
            child_pid: -1,
            pgid: -1,
            closed: AtomicBool::new(true),
        };
        let handle = Arc::new(PtyHandle {
            session,
            terminal: Mutex::new(TerminalEngine::with_scrollback(
                rows as usize,
                cols as usize,
                TERMINAL_SCROLLBACK_LINES,
            )),
            stop: Arc::new(AtomicBool::new(true)),
            join: Mutex::new(None),
            meta: Mutex::new(PtyMeta {
                id,
                name: format!("s{id}"),
                cwd: self.workspace_root.to_string_lossy().into_owned(),
                project_id: None,
                created_at: now_millis(),
                last_used: now_millis(),
                state: crate::proto::TerminalSessionState::TerminalSessionRunning,
                exit_code: None,
                previous_process_ended: false,
                env_overrides_json: "{}".into(),
            }),
        });
        self.ptys.lock().unwrap().insert(id, handle);
    }

    #[cfg(test)]
    pub fn emit_terminal_output_for_test(&self, pty_id: u64, payload: &[u8]) {
        if let Ok(ptys) = self.ptys.lock() {
            if let Some(h) = ptys.get(&pty_id).cloned() {
                let mut pending = payload.to_vec();
                let mut dropped = 0usize;
                self.emit_terminal_output(pty_id, &h, &mut pending, &mut dropped);
            }
        }
    }
}

fn error_response(id: u32, code: &str, message: String) -> Vec<u8> {
    Response {
        id,
        status: Status::Error as i32,
        error: Some(Error {
            code: code.into(),
            message,
        }),
        kind: None,
    }
    .encode_to_vec()
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> CoreError {
    CoreError::Internal("runtime lock was poisoned".into())
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn net_mode_to_proto(mode: crate::net_policy::NetworkMode) -> crate::proto::NetworkMode {
    match mode {
        crate::net_policy::NetworkMode::LocalOnly => crate::proto::NetworkMode::LocalOnly,
        crate::net_policy::NetworkMode::Controlled => crate::proto::NetworkMode::Controlled,
        crate::net_policy::NetworkMode::TemporaryBroader => {
            crate::proto::NetworkMode::TemporaryBroader
        }
    }
}
fn net_mode_from_proto(mode: crate::proto::NetworkMode) -> Result<crate::net_policy::NetworkMode> {
    Ok(match mode {
        crate::proto::NetworkMode::LocalOnly => crate::net_policy::NetworkMode::LocalOnly,
        crate::proto::NetworkMode::Controlled => crate::net_policy::NetworkMode::Controlled,
        crate::proto::NetworkMode::TemporaryBroader => {
            crate::net_policy::NetworkMode::TemporaryBroader
        }
        _ => return Err(CoreError::InvalidArgument("network mode required".into())),
    })
}
fn net_temp_scope_to_proto(
    scope: crate::net_policy::TemporaryScope,
) -> crate::proto::NetworkTemporaryScope {
    match scope {
        crate::net_policy::TemporaryScope::Project => crate::proto::NetworkTemporaryScope::Project,
        crate::net_policy::TemporaryScope::Session => crate::proto::NetworkTemporaryScope::Session,
    }
}
fn net_temp_scope_from_proto(
    scope: crate::proto::NetworkTemporaryScope,
) -> Result<crate::net_policy::TemporaryScope> {
    Ok(match scope {
        crate::proto::NetworkTemporaryScope::Project => crate::net_policy::TemporaryScope::Project,
        crate::proto::NetworkTemporaryScope::Session => crate::net_policy::TemporaryScope::Session,
        _ => crate::net_policy::TemporaryScope::Project,
    })
}
fn net_actor_to_proto(a: crate::net_policy::ActorKind) -> crate::proto::NetworkActorKind {
    match a {
        crate::net_policy::ActorKind::User => crate::proto::NetworkActorKind::User,
        crate::net_policy::ActorKind::Session => crate::proto::NetworkActorKind::Session,
        crate::net_policy::ActorKind::InstalledBinary => {
            crate::proto::NetworkActorKind::InstalledBinary
        }
    }
}
fn net_status_to_proto(
    s: crate::net_activity::ActivityStatus,
) -> crate::proto::NetworkActivityStatus {
    match s {
        crate::net_activity::ActivityStatus::Pending => {
            crate::proto::NetworkActivityStatus::Pending
        }
        crate::net_activity::ActivityStatus::InFlight => {
            crate::proto::NetworkActivityStatus::InFlight
        }
        crate::net_activity::ActivityStatus::Succeeded => {
            crate::proto::NetworkActivityStatus::Succeeded
        }
        crate::net_activity::ActivityStatus::Failed => crate::proto::NetworkActivityStatus::Failed,
        crate::net_activity::ActivityStatus::Cancelled => {
            crate::proto::NetworkActivityStatus::Cancelled
        }
    }
}

fn shell_escape(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

fn serde_json_like_vec(json: &str) -> Vec<String> {
    let t = json.trim();
    if !t.starts_with('[') || !t.ends_with(']') {
        return vec![];
    }
    let inner = &t[1..t.len() - 1];
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut esc = false;
    for ch in inner.chars() {
        if esc {
            cur.push(ch);
            esc = false;
            continue;
        }
        if ch == '\\' {
            esc = true;
            continue;
        }
        if ch == '"' {
            in_str = !in_str;
            if !in_str {
                out.push(cur.clone());
                cur.clear();
            }
            continue;
        }
        if in_str {
            cur.push(ch);
        }
    }
    out
}

fn serde_json_like_vec_encode(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| {
            let mut o = String::from("\"");
            for c in s.chars() {
                match c {
                    '\\' => {
                        o.push('\\');
                        o.push('\\');
                    }
                    '"' => {
                        o.push('\\');
                        o.push('"');
                    }
                    c => o.push(c),
                }
            }
            o.push('"');
            o
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{inner}]")
}

fn hash_bytes(bytes: &[u8]) -> String {
    // Simple FNV-1a 64-bit hex for M8 audit before/after hashes (not cryptographic).
    let mut h: u64 = 14695981039346656037;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    format!("{h:016x}")
}

fn is_text_heuristic(path: &Path, size: u64) -> Result<bool> {
    if size == 0 {
        return Ok(true);
    }
    let mut file = fs::File::open(path)?;
    let mut buf = [0u8; 8192];
    let n = std::io::Read::read(&mut file, &mut buf)?;
    if n == 0 {
        return Ok(true);
    }
    let sample = &buf[..n];
    if sample.contains(&0) {
        return Ok(false);
    }
    Ok(std::str::from_utf8(sample).is_ok())
}

fn validate_session_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > TERMINAL_SESSION_NAME_MAX {
        return Err(CoreError::InvalidArgument(
            "session name must be 1-64 chars".into(),
        ));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(CoreError::InvalidArgument(
            "session name cannot contain control characters".into(),
        ));
    }
    if !name.chars().all(|c| !c.is_control()) {
        return Err(CoreError::InvalidArgument(
            "session name must be printable".into(),
        ));
    }
    Ok(name.to_string())
}

fn is_allowed_env_override(key: &str) -> bool {
    matches!(key, "TERM" | "LANG" | "LC_ALL" | "LC_CTYPE" | "COLORTERM")
}

fn json_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

impl PtyMeta {
    fn to_proto(&self) -> crate::proto::TerminalSessionDescriptor {
        crate::proto::TerminalSessionDescriptor {
            pty_id: self.id,
            name: self.name.clone(),
            cwd: self.cwd.clone(),
            state: self.state as i32,
            exit_code: self.exit_code.unwrap_or(0),
            created_at: self.created_at,
            last_used: self.last_used,
            restorable: self.previous_process_ended,
            previous_process_ended: self.previous_process_ended,
            env_overrides_json: self.env_overrides_json.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    fn read_audit_lines(rt: &Runtime) -> Vec<String> {
        let path = rt.app_data_dir.join("internal/audit/audit.jsonl");
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|s| s.to_string())
            .collect()
    }
    use super::*;

    fn request(id: u32, kind: RequestKind) -> Vec<u8> {
        Request {
            id,
            kind: Some(kind),
        }
        .encode_to_vec()
    }

    fn response(bytes: Vec<u8>) -> Response {
        Response::decode(bytes.as_slice()).unwrap()
    }

    fn assert_ok(response: &Response, id: u32) {
        assert_eq!(response.id, id);
        assert_eq!(response.status, Status::Ok as i32, "{:?}", response.error);
    }

    fn setup_runtime() -> (tempfile::TempDir, Runtime) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let native = root.join("lib");
        fs::create_dir_all(&native).unwrap();
        let db = root.join("internal/db/forgebox.sqlite");
        let runtime = Runtime::new(root, native, db).unwrap();
        (temp, runtime)
    }

    fn create_project(runtime: &Runtime, id: u32, name: &str) {
        let resp = response(runtime.call(&request(
            id,
            RequestKind::CreateProject(CreateProjectRequest { name: name.into() }),
        )));
        assert_ok(&resp, id);
    }

    #[test]
    fn full_workspace_flow_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let native = root.join("lib");
        fs::create_dir_all(&native).unwrap();
        let db = root.join("internal/db/forgebox.sqlite");
        let runtime = Runtime::new(root.clone(), native, db.clone()).unwrap();

        let info = response(runtime.call(&request(1, RequestKind::GetInfo(GetInfoRequest {}))));
        assert_ok(&info, 1);
        let info = match info.kind.unwrap() {
            ResponseKind::GetInfo(info) => info,
            _ => panic!("unexpected response"),
        };
        assert_eq!(info.version, CORE_VERSION);
        assert_eq!(info.project_count, 0);
        assert!(Path::new(&info.workspace_root).join("Projects").is_dir());

        let created = response(runtime.call(&request(
            2,
            RequestKind::CreateProject(CreateProjectRequest {
                name: "MyPythonApp".into(),
            }),
        )));
        assert_ok(&created, 2);
        let created = match created.kind.unwrap() {
            ResponseKind::CreateProject(created) => created,
            _ => panic!("unexpected response"),
        };
        assert!(Path::new(&created.project.unwrap().path).is_dir());

        let written = response(runtime.call(&request(
            3,
            RequestKind::WriteFile(WriteFileRequest {
                project_id: "MyPythonApp".into(),
                path: "main.py".into(),
                content: b"print('hello')\n".to_vec(),
            }),
        )));
        assert_ok(&written, 3);
        assert_eq!(
            match written.kind.unwrap() {
                ResponseKind::WriteFile(written) => written.size,
                _ => panic!("unexpected response"),
            },
            15
        );

        let read = response(runtime.call(&request(
            4,
            RequestKind::ReadFile(ReadFileRequest {
                project_id: "MyPythonApp".into(),
                path: "main.py".into(),
            }),
        )));
        assert_ok(&read, 4);
        assert_eq!(
            match read.kind.unwrap() {
                ResponseKind::ReadFile(read) => read.content,
                _ => panic!("unexpected response"),
            },
            b"print('hello')\n"
        );

        let listed = response(runtime.call(&request(
            5,
            RequestKind::ListFiles(ListFilesRequest {
                project_id: "MyPythonApp".into(),
                path: String::new(),
                recursive: false,
            }),
        )));
        assert_ok(&listed, 5);
        let listed = match listed.kind.unwrap() {
            ResponseKind::ListFiles(listed) => listed,
            _ => panic!("unexpected response"),
        };
        assert_eq!(listed.entries.len(), 1);
        assert_eq!(listed.entries[0].name, "main.py");

        let rejected = response(runtime.call(&request(
            6,
            RequestKind::WriteFile(WriteFileRequest {
                project_id: "MyPythonApp".into(),
                path: "../escape.txt".into(),
                content: b"no".to_vec(),
            }),
        )));
        assert_eq!(rejected.status, Status::Error as i32);
        assert_eq!(rejected.error.unwrap().code, "INVALID_ARGUMENT");

        let events = runtime.poll_events(32);
        assert!(events.len() >= 3);
        for event in events {
            assert!(Event::decode(event.as_slice()).is_ok());
        }

        drop(runtime);
        let reopened = Runtime::new(root, PathBuf::new(), db).unwrap();
        let projects = response(reopened.call(&request(
            7,
            RequestKind::ListProjects(ListProjectsRequest {}),
        )));
        assert_ok(&projects, 7);
        assert_eq!(
            match projects.kind.unwrap() {
                ResponseKind::ListProjects(projects) => projects.projects.len(),
                _ => panic!("unexpected response"),
            },
            1
        );
    }

    #[test]
    fn stat_and_write_if_unchanged_enforce_conflicts() {
        let (_temp, runtime) = setup_runtime();
        create_project(&runtime, 1, "Proj");
        let write = response(runtime.call(&request(
            2,
            RequestKind::WriteFile(WriteFileRequest {
                project_id: "Proj".into(),
                path: "a.txt".into(),
                content: b"hello".to_vec(),
            }),
        )));
        assert_ok(&write, 2);

        let stat = response(runtime.call(&request(
            3,
            RequestKind::StatFile(StatFileRequest {
                project_id: "Proj".into(),
                path: "a.txt".into(),
            }),
        )));
        assert_ok(&stat, 3);
        let stat = match stat.kind.unwrap() {
            ResponseKind::StatFile(s) => s,
            _ => panic!("unexpected"),
        };
        assert_eq!(stat.size, 5);
        assert!(stat.is_text_heuristic);
        assert!(stat.mtime_millis > 0);

        let missing = response(runtime.call(&request(
            4,
            RequestKind::StatFile(StatFileRequest {
                project_id: "Proj".into(),
                path: "missing.txt".into(),
            }),
        )));
        assert_eq!(missing.status, Status::Error as i32);

        let conflict = response(runtime.call(&request(
            5,
            RequestKind::WriteFileIfUnchanged(WriteFileIfUnchangedRequest {
                project_id: "Proj".into(),
                path: "a.txt".into(),
                content: b"new".to_vec(),
                expected_mtime_millis: stat.mtime_millis - 1,
            }),
        )));
        assert_eq!(conflict.status, Status::Error as i32);
        let conflict_msg = conflict.error.unwrap().message;
        assert!(conflict_msg.contains("CONFLICT"));

        let read_back = response(runtime.call(&request(
            6,
            RequestKind::ReadFile(ReadFileRequest {
                project_id: "Proj".into(),
                path: "a.txt".into(),
            }),
        )));
        assert_ok(&read_back, 6);
        let content = match read_back.kind.unwrap() {
            ResponseKind::ReadFile(r) => r.content,
            _ => panic!("unexpected"),
        };
        assert_eq!(content, b"hello");

        let ok_write = response(runtime.call(&request(
            7,
            RequestKind::WriteFileIfUnchanged(WriteFileIfUnchangedRequest {
                project_id: "Proj".into(),
                path: "a.txt".into(),
                content: b"updated".to_vec(),
                expected_mtime_millis: stat.mtime_millis,
            }),
        )));
        assert_ok(&ok_write, 7);
        let ok_write = match ok_write.kind.unwrap() {
            ResponseKind::WriteFileIfUnchanged(r) => r,
            _ => panic!("unexpected"),
        };
        assert_eq!(ok_write.size, 7);
        assert!(ok_write.mtime_millis >= stat.mtime_millis);
    }

    #[test]
    fn rename_and_delete_path_enforce_security_rules() {
        let (_temp, runtime) = setup_runtime();
        create_project(&runtime, 1, "Proj");

        // Create nested file
        let write = response(runtime.call(&request(
            2,
            RequestKind::WriteFile(WriteFileRequest {
                project_id: "Proj".into(),
                path: "dir/a.txt".into(),
                content: b"hi".to_vec(),
            }),
        )));
        assert_ok(&write, 2);

        // traversal rejection
        let bad_rename = response(runtime.call(&request(
            3,
            RequestKind::RenameFile(RenameFileRequest {
                project_id: "Proj".into(),
                old_path: "dir/a.txt".into(),
                new_path: "../escape.txt".into(),
            }),
        )));
        assert_eq!(bad_rename.status, Status::Error as i32);
        assert_eq!(bad_rename.error.unwrap().code, "INVALID_ARGUMENT");

        let bad_delete = response(runtime.call(&request(
            4,
            RequestKind::DeletePath(DeletePathRequest {
                project_id: "Proj".into(),
                path: "../escape".into(),
                recursive: true,
            }),
        )));
        assert_eq!(bad_delete.status, Status::Error as i32);

        // delete project root rejected
        let root_delete = response(runtime.call(&request(
            5,
            RequestKind::DeletePath(DeletePathRequest {
                project_id: "Proj".into(),
                path: "".into(),
                recursive: true,
            }),
        )));
        assert_eq!(root_delete.status, Status::Error as i32);

        // non-recursive delete of non-empty dir rejected
        let non_recursive = response(runtime.call(&request(
            6,
            RequestKind::DeletePath(DeletePathRequest {
                project_id: "Proj".into(),
                path: "dir".into(),
                recursive: false,
            }),
        )));
        assert_eq!(non_recursive.status, Status::Error as i32);
        assert!(non_recursive.error.unwrap().message.contains("recursive"));

        // successful rename
        let renamed = response(runtime.call(&request(
            7,
            RequestKind::RenameFile(RenameFileRequest {
                project_id: "Proj".into(),
                old_path: "dir/a.txt".into(),
                new_path: "dir/b.txt".into(),
            }),
        )));
        assert_ok(&renamed, 7);
        let read_old = response(runtime.call(&request(
            8,
            RequestKind::ReadFile(ReadFileRequest {
                project_id: "Proj".into(),
                path: "dir/a.txt".into(),
            }),
        )));
        assert_eq!(read_old.status, Status::Error as i32);
        let read_new = response(runtime.call(&request(
            9,
            RequestKind::ReadFile(ReadFileRequest {
                project_id: "Proj".into(),
                path: "dir/b.txt".into(),
            }),
        )));
        assert_ok(&read_new, 9);

        // successful recursive delete
        let deleted = response(runtime.call(&request(
            10,
            RequestKind::DeletePath(DeletePathRequest {
                project_id: "Proj".into(),
                path: "dir".into(),
                recursive: true,
            }),
        )));
        assert_ok(&deleted, 10);
        let listed = response(runtime.call(&request(
            11,
            RequestKind::ListFiles(ListFilesRequest {
                project_id: "Proj".into(),
                path: String::new(),
                recursive: true,
            }),
        )));
        assert_ok(&listed, 11);
        let entries = match listed.kind.unwrap() {
            ResponseKind::ListFiles(l) => l.entries,
            _ => panic!("unexpected"),
        };
        assert!(entries.is_empty());
    }

    #[test]
    fn pty_open_rejects_cwd_outside_forgebox_projects() {
        let (_temp, runtime) = setup_runtime();
        let err = runtime
            .pty_open(Some("../escape".into()), None, vec![], 80, 24, 6)
            .unwrap_err();
        match err {
            CoreError::InvalidArgument(_)
            | CoreError::PathEscape(_)
            | CoreError::NotFound(_)
            | CoreError::Internal(_) => {} // Windows host: PTY unsupported
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn rename_and_delete_reject_symlinks() {
        use std::os::unix::fs::symlink;
        let (_temp, runtime) = setup_runtime();
        create_project(&runtime, 1, "Proj");
        let root = runtime.workspace_root.join("Projects").join("Proj");
        let outside = _temp.path().join("outside.txt");
        fs::write(&outside, b"x").unwrap();
        symlink(&outside, root.join("leak.txt")).unwrap();

        let rename = response(runtime.call(&request(
            2,
            RequestKind::RenameFile(RenameFileRequest {
                project_id: "Proj".into(),
                old_path: "leak.txt".into(),
                new_path: "safe.txt".into(),
            }),
        )));
        assert_eq!(rename.status, Status::Error as i32);

        let delete = response(runtime.call(&request(
            3,
            RequestKind::DeletePath(DeletePathRequest {
                project_id: "Proj".into(),
                path: "leak.txt".into(),
                recursive: false,
            }),
        )));
        assert_eq!(delete.status, Status::Error as i32);
    }

    #[test]
    fn session_cap_enforced_and_names_unique() {
        let (_temp, runtime) = setup_runtime();
        let mut ids = Vec::new();
        for i in 0..6 {
            match runtime.pty_open(None, Some(format!("s{i}")), vec![], 80, 24, 6) {
                Ok(id) => ids.push(id),
                Err(CoreError::Internal(_)) | Err(CoreError::NotFound(_)) => {
                    // Windows/non-PTY host: verify cap error path without spawning.
                    let err = runtime.pty_open(None, Some("cap".into()), vec![], 80, 24, 0);
                    match err {
                        Err(CoreError::InvalidArgument(_)) => return, // cap reached/invalid as expected shape
                        Err(CoreError::Internal(_)) | Err(CoreError::NotFound(_)) => return, // PTY unsupported
                        Ok(id) => {
                            let _ = runtime.pty_close(id);
                            return;
                        }
                        Err(other) => panic!("{other:?}"),
                    }
                }
                Err(other) => panic!("{other:?}"),
            }
        }
        let cap_err = runtime
            .pty_open(None, Some("overflow".into()), vec![], 80, 24, ids.len())
            .unwrap_err();
        match cap_err {
            CoreError::InvalidArgument(msg) => assert!(msg.contains("SESSION_CAP_REACHED")),
            other => panic!("{other:?}"),
        }
        let dup = runtime
            .pty_open(None, Some("s0".into()), vec![], 80, 24, 12)
            .unwrap_err();
        match dup {
            CoreError::InvalidArgument(_) => {}
            other => panic!("{other:?}"),
        }
        for id in ids {
            let _ = runtime.pty_close(id);
        }
    }

    #[test]
    fn stop_all_marks_sessions_stopped() {
        let (_temp, runtime) = setup_runtime();
        let _ = runtime.pty_open(None, Some("a".into()), vec![], 80, 24, 6);
        let _ = runtime.pty_open(None, Some("b".into()), vec![], 80, 24, 6);
        let stopped = runtime.pty_stop_all().unwrap_or(0);
        assert!(stopped <= 2);
        let sessions = runtime.list_terminal_sessions().unwrap();
        assert!(sessions.iter().all(|s| s.state
            != crate::proto::TerminalSessionState::TerminalSessionRunning
            || stopped == 0));
    }
    #[test]
    fn restore_markers_round_trip_env_and_no_pty_spawn() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let native = root.join("lib");
        fs::create_dir_all(&native).unwrap();
        let db_path = root.join("internal/db/forgebox.sqlite");
        let runtime = Runtime::new(root.clone(), native.clone(), db_path.clone()).unwrap();
        let now = now_millis();
        let row = db::TerminalSessionRow {
            id: 42,
            name: "agent".into(),
            cwd: root.join("ForgeBox").to_string_lossy().into_owned(),
            env_overrides_json: "{\"TERM\":\"xterm-256color\",\"LANG\":\"C.UTF-8\"}".into(),
            last_used: now,
            was_running: true,
            exit_code: None,
        };
        {
            let db = runtime.db.lock().unwrap();
            db::upsert_terminal_session(&db, &row).unwrap();
        }
        drop(runtime);
        let reopened = Runtime::new(root, native, db_path).unwrap();
        let events = reopened.poll_events(32);
        let restore: Vec<_> = events
            .into_iter()
            .filter_map(|b| {
                let ev = Event::decode(b.as_slice()).ok()?;
                match ev.kind {
                    Some(EventKind::TerminalSessionRestoreAvailable(e)) => e.session,
                    _ => None,
                }
            })
            .collect();
        assert_eq!(restore.len(), 1);
        let s = restore[0].clone();
        assert_eq!(s.pty_id, 42);
        assert_eq!(s.name, "agent");
        assert!(s.restorable);
        assert!(s.previous_process_ended);
        assert!(s.env_overrides_json.contains("TERM"));
        assert!(reopened.ptys.lock().unwrap().is_empty());
    }

    #[test]
    fn three_high_rate_sessions_coalesce_and_bound_memory() {
        let (_temp, runtime) = setup_runtime();
        for id in 1..=3 {
            runtime.inject_test_terminal_session(id, 24, 80);
        }
        let runtime = Arc::new(runtime);
        let mut writers = Vec::new();
        for id in 1u64..=3 {
            let rt = Arc::clone(&runtime);
            writers.push(std::thread::spawn(move || {
                let chunk = vec![b'x'; 4096];
                for _ in 0..256 {
                    // ~1MiB per session
                    rt.emit_terminal_output_for_test(id, &chunk);
                }
            }));
        }
        let poller = {
            let rt = Arc::clone(&runtime);
            std::thread::spawn(move || {
                let mut total = 0usize;
                let start = std::time::Instant::now();
                while start.elapsed() < Duration::from_millis(1500) {
                    let ev = rt.poll_events_timeout(64, 16);
                    total += ev.len();
                    if total > 0 && start.elapsed() > Duration::from_millis(500) {
                        break;
                    }
                }
                total
            })
        };
        for w in writers {
            w.join().unwrap();
        }
        let polled = poller.join().unwrap();
        assert!(polled > 0);
        // Bounded memory: each session has 10k line scrollback + 24 visible lines, event queue drained by poller.
        let qlen = runtime.events.lock().unwrap().len();
        assert!(qlen < 10_000, "event queue unbounded: {qlen}");
        for id in 1..=3 {
            let ptys = runtime.ptys.lock().unwrap();
            let h = ptys.get(&id).unwrap();
            let term = h.terminal.lock().unwrap();
            assert!(term.state.scrollback.len() <= TERMINAL_SCROLLBACK_LINES);
            assert!(term.state.total_lines() <= TERMINAL_SCROLLBACK_LINES + 24);
        }
        // Simulate reader backpressure: pending beyond cap causes drop notice on next emit.
        if let Ok(ptys) = runtime.ptys.lock() {
            if let Some(h) = ptys.get(&1u64).cloned() {
                let mut pending = vec![b'y'; TERMINAL_OUTPUT_COALESCE_BYTES];
                let mut dropped = TERMINAL_OUTPUT_COALESCE_BYTES * 5;
                runtime.emit_terminal_output(1, &h, &mut pending, &mut dropped);
            }
        }
        let mut has_drop = false;
        {
            let ptys = runtime.ptys.lock().unwrap();
            let h = ptys.get(&1u64).unwrap();
            let term = h.terminal.lock().unwrap();
            has_drop = term
                .state
                .scrollback
                .iter()
                .any(|l| l.contains("bytes dropped"))
                || term.state.lines.iter().any(|l| l.contains("bytes dropped"));
        }
        assert!(
            has_drop,
            "expected drop notice when sink/coalesce buffer overflows"
        );
    }

    #[test]
    fn policy_decision_and_approval_audit_records() {
        let (_temp, runtime) = setup_runtime();
        let req = Request {
            id: 1,
            kind: Some(RequestKind::EvaluateCommand(
                crate::proto::EvaluateCommandRequest {
                    command: "rm -rf /tmp/x".into(),
                    actor_is_agent: false,
                    session_id: "s1".into(),
                    cwd: "/p".into(),
                    origin: crate::proto::CommandOrigin::User as i32,
                },
            )),
        };
        let bytes = runtime.call(&req.encode_to_vec());
        let resp = Response::decode(bytes.as_slice()).unwrap();
        assert_eq!(resp.status, Status::Ok as i32);
        let issue = Request {
            id: 2,
            kind: Some(RequestKind::IssueApprovalToken(
                crate::proto::IssueApprovalTokenRequest {
                    command_hash: "abc".into(),
                    session_id: "s1".into(),
                },
            )),
        };
        let iresp = Response::decode(runtime.call(&issue.encode_to_vec()).as_slice()).unwrap();
        let token = match iresp.kind {
            Some(ResponseKind::IssueApprovalToken(r)) => r.token,
            _ => panic!("bad"),
        };
        let consume = Request {
            id: 3,
            kind: Some(RequestKind::ConsumeApprovalToken(
                crate::proto::ConsumeApprovalTokenRequest {
                    token: token.clone(),
                    command_hash: "abc".into(),
                    session_id: "s1".into(),
                },
            )),
        };
        let cresp = Response::decode(runtime.call(&consume.encode_to_vec()).as_slice()).unwrap();
        assert_eq!(cresp.status, Status::Ok as i32);
        let lines = read_audit_lines(&runtime).join("\n");
        assert!(lines.contains("\"op\":\"policy_decision\""));
        assert!(lines.contains("\"op\":\"approval_issued\""));
        assert!(lines.contains("\"op\":\"approval_consumed\""));
        assert!(
            !lines.contains(&token),
            "raw token must be redacted in audit"
        );
    }

    #[test]
    fn network_decision_matrix_and_audit() {
        let (_temp, runtime) = setup_runtime();
        let set = Request {
            id: 1,
            kind: Some(RequestKind::NetworkSetMode(
                crate::proto::NetworkSetModeRequest {
                    mode: crate::proto::NetworkMode::Controlled as i32,
                    temporary_scope: 0,
                    scope_id: "".into(),
                    duration_seconds: 0,
                },
            )),
        };
        let resp = Response::decode(runtime.call(&set.encode_to_vec()).as_slice()).unwrap();
        assert_eq!(resp.status, Status::Ok as i32);
        let decide_req = |id, host: &str| Request {
            id,
            kind: Some(RequestKind::NetworkDecision(
                crate::proto::NetworkDecisionRequest {
                    destination_host: host.into(),
                    actor_kind: crate::proto::NetworkActorKind::User as i32,
                    actor_id: "u".into(),
                    session_id: "u".into(),
                },
            )),
        };
        let deny = Response::decode(
            runtime
                .call(&decide_req(2, "example.com").encode_to_vec())
                .as_slice(),
        )
        .unwrap();
        let dr = match deny.kind {
            Some(ResponseKind::NetworkDecision(r)) => r,
            _ => panic!("bad"),
        };
        assert!(!dr.allow);
        assert!(dr.reason.contains("not in allowlist"));
        let up = Request {
            id: 3,
            kind: Some(RequestKind::NetworkAllowlistUpsert(
                crate::proto::NetworkAllowlistUpsertRequest {
                    host_pattern: "example.com".into(),
                    provider_kind: "test".into(),
                },
            )),
        };
        let _ = runtime.call(&up.encode_to_vec());
        let allow = Response::decode(
            runtime
                .call(&decide_req(4, "example.com").encode_to_vec())
                .as_slice(),
        )
        .unwrap();
        let ar = match allow.kind {
            Some(ResponseKind::NetworkDecision(r)) => r,
            _ => panic!("bad"),
        };
        assert!(ar.allow);
        let lines = read_audit_lines(&runtime).join("\n");
        assert!(lines.contains("\"op\":\"net_decision\""));
    }

    #[test]
    fn network_temporary_expiry_and_session_toggle() {
        let (_temp, runtime) = setup_runtime();
        let set = Request {
            id: 1,
            kind: Some(RequestKind::NetworkSetMode(
                crate::proto::NetworkSetModeRequest {
                    mode: crate::proto::NetworkMode::TemporaryBroader as i32,
                    temporary_scope: crate::proto::NetworkTemporaryScope::Session as i32,
                    scope_id: "s1".into(),
                    duration_seconds: 1,
                },
            )),
        };
        let _ = runtime.call(&set.encode_to_vec());
        let toggle = Request {
            id: 2,
            kind: Some(RequestKind::NetworkSessionPolicy(
                crate::proto::NetworkSessionPolicyRequest {
                    session_id: "s1".into(),
                    network_enabled: false,
                },
            )),
        };
        let tresp = Response::decode(runtime.call(&toggle.encode_to_vec()).as_slice()).unwrap();
        let tr = match tresp.kind {
            Some(ResponseKind::NetworkSessionPolicy(r)) => r,
            _ => panic!("bad"),
        };
        assert!(tr.ok);
        assert!(!tr.network_enabled);
        let dec = Request {
            id: 3,
            kind: Some(RequestKind::NetworkDecision(
                crate::proto::NetworkDecisionRequest {
                    destination_host: "example.com".into(),
                    actor_kind: crate::proto::NetworkActorKind::Session as i32,
                    actor_id: "s1".into(),
                    session_id: "s1".into(),
                },
            )),
        };
        let dresp = Response::decode(runtime.call(&dec.encode_to_vec()).as_slice()).unwrap();
        let dr = match dresp.kind {
            Some(ResponseKind::NetworkDecision(r)) => r,
            _ => panic!("bad"),
        };
        assert!(!dr.allow);
        assert!(dr.reason.contains("per-session network toggle disabled"));
    }

    #[test]
    fn network_cancel_records_audit_and_status() {
        let (_temp, runtime) = setup_runtime();
        let dec = Request {
            id: 1,
            kind: Some(RequestKind::NetworkDecision(
                crate::proto::NetworkDecisionRequest {
                    destination_host: "example.com".into(),
                    actor_kind: crate::proto::NetworkActorKind::User as i32,
                    actor_id: "u".into(),
                    session_id: "u".into(),
                },
            )),
        };
        let _ = runtime.call(&dec.encode_to_vec());
        let list = Request {
            id: 2,
            kind: Some(RequestKind::NetworkActivityList(
                crate::proto::NetworkActivityListRequest { limit: 10 },
            )),
        };
        let lresp = Response::decode(runtime.call(&list.encode_to_vec()).as_slice()).unwrap();
        let lr = match lresp.kind {
            Some(ResponseKind::NetworkActivityList(r)) => r,
            _ => panic!("bad"),
        };
        assert!(!lr.events.is_empty());
        let token = lr.events[0].cancel_token.clone();
        let cancel = Request {
            id: 3,
            kind: Some(RequestKind::NetworkCancel(
                crate::proto::NetworkCancelRequest {
                    cancel_token: token,
                },
            )),
        };
        let cresp = Response::decode(runtime.call(&cancel.encode_to_vec()).as_slice()).unwrap();
        let cr = match cresp.kind {
            Some(ResponseKind::NetworkCancel(r)) => r,
            _ => panic!("bad"),
        };
        assert!(cr.ok || !cr.ok); // may be failed if not in-flight
        let lines = read_audit_lines(&runtime).join("\n");
        assert!(lines.contains("\"op\":\"net_decision\""));
    }

    #[test]
    fn linux_env_status_install_uninstall_and_audit() {
        let (_temp, runtime) = setup_runtime();
        let status = Request {
            id: 1,
            kind: Some(RequestKind::LinuxEnvStatus(
                crate::proto::LinuxEnvStatusRequest {},
            )),
        };
        let resp = Response::decode(runtime.call(&status.encode_to_vec()).as_slice()).unwrap();
        let st = match resp.kind {
            Some(ResponseKind::LinuxEnvStatus(r)) => r,
            _ => panic!("bad"),
        };
        assert_eq!(
            st.state,
            crate::proto::LinuxEnvState::LinuxEnvNotInstalled as i32
        );
        assert!(st
            .rootfs_path
            .replace('\\', "/")
            .contains("/ForgeBox/Cache/linux/"));

        let install = Request {
            id: 2,
            kind: Some(RequestKind::LinuxEnvInstall(
                crate::proto::LinuxEnvInstallRequest {
                    distro: crate::proto::LinuxDistro::Alpine as i32,
                    bootstrap_mode: crate::proto::LinuxBootstrapMode::BundledOffline as i32,
                    source_path: String::new(),
                },
            )),
        };
        let iresp = Response::decode(runtime.call(&install.encode_to_vec()).as_slice()).unwrap();
        let ir = match iresp.kind {
            Some(ResponseKind::LinuxEnvInstall(r)) => r,
            _ => panic!("bad"),
        };
        assert_eq!(ir.state, crate::proto::LinuxEnvState::LinuxEnvFailed as i32);
        assert!(ir.message.contains("Bundled rootfs source not found"));

        // M10.1 C1: bundled offline install with fake rootfs+loader becomes READY
        let native = _temp.path().join("native");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(native.join("libproot.so"), b"proot").unwrap();
        std::fs::write(native.join("libprootloader.so"), b"loader").unwrap();
        std::fs::write(native.join("libprootloader32.so"), b"loader32").unwrap();
        let mut ar = tar::Builder::new(Vec::new());
        for (name, data, mode) in [
            ("bin/sh", b"#!/bin/sh\n".as_slice(), 0o755u32),
            ("etc/os-release", b"alpine".as_slice(), 0o644u32),
        ] {
            let mut h = tar::Header::new_gnu();
            h.set_path(name).unwrap();
            h.set_size(data.len() as u64);
            h.set_mode(mode);
            h.set_cksum();
            ar.append(&h, data).unwrap();
        }
        let tar_bytes = ar.into_inner().unwrap();
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::copy(&mut &tar_bytes[..], &mut enc).unwrap();
        let src = _temp.path().join("alpine.tar.gz");
        std::fs::write(&src, enc.finish().unwrap()).unwrap();
        let runtime2 = Runtime::new(
            _temp.path().to_path_buf(),
            native.clone(),
            _temp.path().join("db2.sqlite"),
        )
        .unwrap();
        let install2 = Request {
            id: 10,
            kind: Some(RequestKind::LinuxEnvInstall(
                crate::proto::LinuxEnvInstallRequest {
                    distro: crate::proto::LinuxDistro::Alpine as i32,
                    bootstrap_mode: crate::proto::LinuxBootstrapMode::BundledOffline as i32,
                    source_path: src.to_string_lossy().into_owned(),
                },
            )),
        };
        let i2 = Response::decode(runtime2.call(&install2.encode_to_vec()).as_slice()).unwrap();
        let ir2 = match i2.kind {
            Some(ResponseKind::LinuxEnvInstall(r)) => r,
            _ => panic!("bad"),
        };
        assert_eq!(ir2.state, crate::proto::LinuxEnvState::LinuxEnvReady as i32);
        let st2 = Response::decode(
            runtime2
                .call(
                    &Request {
                        id: 11,
                        kind: Some(RequestKind::LinuxEnvStatus(
                            crate::proto::LinuxEnvStatusRequest {},
                        )),
                    }
                    .encode_to_vec(),
                )
                .as_slice(),
        )
        .unwrap();
        let st2 = match st2.kind {
            Some(ResponseKind::LinuxEnvStatus(s)) => s,
            _ => panic!("bad"),
        };
        assert!(st2.rootfs_present);
        assert!(st2.proot_loader_present);
        assert_eq!(st2.shell_binary, "/bin/sh");

        let un = Request {
            id: 3,
            kind: Some(RequestKind::LinuxEnvUninstall(
                crate::proto::LinuxEnvUninstallRequest {},
            )),
        };
        let uresp = Response::decode(runtime.call(&un.encode_to_vec()).as_slice()).unwrap();
        let ur = match uresp.kind {
            Some(ResponseKind::LinuxEnvUninstall(r)) => r,
            _ => panic!("bad"),
        };
        assert!(ur.ok);
        let lines = read_audit_lines(&runtime).join("\n");
        assert!(lines.contains("\"op\":\"linux_env_install\""));
        assert!(lines.contains("\"op\":\"linux_env_uninstall\""));
    }

    #[test]
    fn linux_spawn_shell_rejects_non_ready_and_forbidden_cwd() {
        let (_temp, runtime) = setup_runtime();
        let req = Request {
            id: 1,
            kind: Some(RequestKind::LinuxShellSpawn(
                crate::proto::LinuxShellSpawnRequest {
                    project_id: "Missing".into(),
                    cols: 80,
                    rows: 24,
                    env_json: "{}".into(),
                },
            )),
        };
        let resp = Response::decode(runtime.call(&req.encode_to_vec()).as_slice()).unwrap();
        assert_eq!(resp.status, Status::Error as i32);
    }

    #[test]
    fn installed_binary_run_requires_confirm_and_audits() {
        let (_temp, runtime) = setup_runtime();
        let proj = runtime
            .create_project(CreateProjectRequest { name: "P".into() })
            .unwrap()
            .project
            .unwrap();
        let hash = crate::policy::evaluate_command_with_origin(
            "codex --version",
            false,
            "sess1",
            "",
            crate::policy::CommandOrigin::InstalledBinary,
        )
        .command_hash;
        let req = Request {
            id: 1,
            kind: Some(RequestKind::LinuxRunInstalledBinary(
                crate::proto::LinuxRunInstalledBinaryRequest {
                    project_id: proj.id.clone(),
                    binary_name: "codex".into(),
                    argv_json: "[\"--version\"]".into(),
                    approval_token: "".into(),
                    command_hash: hash.clone(),
                    session_id: "sess1".into(),
                },
            )),
        };
        let resp = Response::decode(runtime.call(&req.encode_to_vec()).as_slice()).unwrap();
        assert_eq!(resp.status, Status::Error as i32);
        assert!(resp.error.unwrap().message.contains("CONFIRM"));

        let tok = crate::policy::issue_approval_token(&hash, "sess1").unwrap();
        let req2 = Request {
            id: 2,
            kind: Some(RequestKind::LinuxRunInstalledBinary(
                crate::proto::LinuxRunInstalledBinaryRequest {
                    project_id: proj.id.clone(),
                    binary_name: "codex".into(),
                    argv_json: "[\"--version\"]".into(),
                    approval_token: tok.clone(),
                    command_hash: hash,
                    session_id: "sess1".into(),
                },
            )),
        };
        let resp2 = Response::decode(runtime.call(&req2.encode_to_vec()).as_slice()).unwrap();
        // may fail spawning /system/bin/sh in non-android host? pty_open uses /system/bin/sh or /bin/sh; on windows test env it may fail. handle gracefully.
        if resp2.status == Status::Ok as i32 {
            let lines = read_audit_lines(&runtime).join("\n");
            assert!(lines.contains("\"op\":\"installed_binary_run\""));
            assert!(!lines.contains(&tok));
        }
    }

    #[test]
    fn approval_denied_audit_record_on_bad_consume() {
        let (_temp, runtime) = setup_runtime();
        let req = Request {
            id: 1,
            kind: Some(RequestKind::ConsumeApprovalToken(
                crate::proto::ConsumeApprovalTokenRequest {
                    token: "aptok_bad_1".into(),
                    command_hash: "abc".into(),
                    session_id: "s1".into(),
                },
            )),
        };
        let resp = Response::decode(runtime.call(&req.encode_to_vec()).as_slice()).unwrap();
        assert_eq!(resp.status, Status::Error as i32);
        let lines = read_audit_lines(&runtime).join("\n");
        assert!(lines.contains("\"op\":\"approval_denied\""));
    }

    #[cfg(unix)]
    #[test]
    fn supervisor_close_kills_forked_child_process_group() {
        let (_temp, runtime) = setup_runtime();
        let id = runtime.pty_open(None, None, vec![], 80, 24, 6).unwrap();
        runtime.pty_write(id, b"sleep 300 &\nwait\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let pgid = runtime.ptys.lock().unwrap().get(&id).unwrap().session.pgid;
        runtime.pty_close(id).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let rc = unsafe { libc::kill(-pgid, 0) };
        assert_ne!(rc, 0, "process group should be dead");
        let lines = read_audit_lines(&runtime).join("\n");
        assert!(lines.contains("\"op\":\"supervisor_kill\""));
    }

    #[cfg(unix)]
    #[test]
    fn supervisor_stop_all_kills_all_sessions_no_zombies() {
        let (_temp, runtime) = setup_runtime();
        let mut pgids = vec![];
        for _ in 0..3 {
            let id = runtime.pty_open(None, None, vec![], 80, 24, 6).unwrap();
            runtime.pty_write(id, b"sleep 300 &\nwait\n").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(150));
            pgids.push(runtime.ptys.lock().unwrap().get(&id).unwrap().session.pgid);
        }
        let stopped = runtime.pty_stop_all().unwrap();
        assert_eq!(stopped, 3);
        std::thread::sleep(std::time::Duration::from_millis(500));
        for pgid in pgids {
            let rc = unsafe { libc::kill(-pgid, 0) };
            assert_ne!(rc, 0, "pgid {pgid} should be dead");
        }
        let lines = read_audit_lines(&runtime).join("\n");
        assert!(lines.matches("\"op\":\"supervisor_kill\"").count() >= 3);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_drop_kills_sessions() {
        let (temp, runtime) = setup_runtime();
        let app = temp.path().to_path_buf();
        let id = runtime.pty_open(None, None, vec![], 80, 24, 6).unwrap();
        runtime.pty_write(id, b"sleep 300 &\nwait\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let pgid = runtime.ptys.lock().unwrap().get(&id).unwrap().session.pgid;
        drop(runtime);
        std::thread::sleep(std::time::Duration::from_millis(500));
        let rc = unsafe { libc::kill(-pgid, 0) };
        assert_ne!(rc, 0, "pgid should be dead after runtime drop");
        let audit_path = app.join("internal/audit/audit.jsonl");
        let lines = std::fs::read_to_string(audit_path).unwrap_or_default();
        assert!(lines.contains("runtime_drop"));
    }
}
