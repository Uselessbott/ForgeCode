use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use prost::Message;
use rusqlite::Connection;

use crate::db;
use crate::error::{CoreError, Result};
use crate::proto::event::Kind as EventKind;
use crate::proto::request::Kind as RequestKind;
use crate::proto::response::Kind as ResponseKind;
use crate::proto::{
    CreateProjectRequest, CreateProjectResponse, Error, Event, FileChangedEvent, FileEntry,
    GetInfoRequest, GetInfoResponse, ListFilesRequest, ListFilesResponse, ListProjectsRequest,
    ListProjectsResponse, LogEvent, Project, ReadFileRequest, ReadFileResponse, Request, Response,
    Status, WriteFileRequest, WriteFileResponse,
};
use crate::vfs;

pub const CORE_VERSION: &str = "0.2.0-m3";

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

pub struct Runtime {
    app_data_dir: PathBuf,
    native_lib_dir: PathBuf,
    db_path: PathBuf,
    workspace_root: PathBuf,
    db: Mutex<Connection>,
    events: Mutex<VecDeque<Vec<u8>>>,
    next_seq: AtomicU64,
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
        let runtime = Self {
            app_data_dir,
            native_lib_dir,
            db_path,
            workspace_root,
            db: Mutex::new(db),
            events: Mutex::new(VecDeque::new()),
            next_seq: AtomicU64::new(next_seq),
        };
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
        let mut queue = match self.events.lock() {
            Ok(queue) => queue,
            Err(_) => return Vec::new(),
        };
        let count = max.min(queue.len());
        (0..count).filter_map(|_| queue.pop_front()).collect()
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
            None => Err(CoreError::InvalidArgument("missing request kind".into())),
        }
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
        fs::write(&target, &request.content)?;
        let size = fs::metadata(&target)?.len();

        {
            let db = self.db.lock().map_err(lock_error)?;
            db::upsert_file_index(&db, &request.project_id, &request.path, size, now_millis())?;
        }
        self.emit_file_event(
            &request.project_id,
            &request.path,
            if existed { "modified" } else { "created" },
        )?;
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

    fn list_files(&self, request: ListFilesRequest) -> Result<ListFilesResponse> {
        self.assert_project_exists(&request.project_id)?;
        let relative = vfs::validate_relative_path(&request.path, true)?;
        let project_dir = self.project_dir(&request.project_id)?;
        let directory = vfs::resolve(&project_dir, &relative, true)?;
        if !directory.is_dir() {
            return Err(CoreError::InvalidArgument("path is not a directory".into()));
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let child_relative = relative.join(&name).to_string_lossy().into_owned();
            entries.push(FileEntry {
                name,
                path: child_relative,
                is_dir: file_type.is_dir(),
                size: if file_type.is_file() {
                    entry.metadata()?.len()
                } else {
                    0
                },
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(ListFilesResponse { entries })
    }

    fn project_dir(&self, project_id: &str) -> Result<PathBuf> {
        vfs::validate_project_name(project_id)?;
        Ok(self.workspace_root.join("Projects").join(project_id))
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
        self.events.lock().map_err(lock_error)?.push_back(payload);
        Ok(())
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

#[cfg(test)]
mod tests {
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
}
