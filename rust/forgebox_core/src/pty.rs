#![cfg_attr(not(unix), allow(dead_code))]

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use crate::error::{CoreError, Result};

#[derive(Clone, Debug)]
pub struct PtyConfig {
    pub shell_path: PathBuf,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug)]
pub struct PtySession {
    pub id: u64,
    pub master_fd: i32,
    pub child_pid: i32,
    pub pgid: i32,
    pub(crate) closed: AtomicBool,
}

#[cfg(unix)]
mod unix_impl {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::RawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::{Duration, Instant};

    pub fn spawn(config: &PtyConfig) -> Result<PtySession> {
        if !config.shell_path.exists() {
            return Err(CoreError::NotFound(format!(
                "shell not found: {}",
                config.shell_path.display()
            )));
        }
        if !config.cwd.is_dir() {
            return Err(CoreError::NotFound(format!(
                "cwd not found: {}",
                config.cwd.display()
            )));
        }

        unsafe {
            let mut master: RawFd = -1;
            let mut slave: RawFd = -1;
            let mut win = libc::winsize {
                ws_row: config.rows.max(1),
                ws_col: config.cols.max(1),
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            if libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut win,
            ) != 0
            {
                return Err(CoreError::Io(std::io::Error::last_os_error()));
            }

            let pid = libc::fork();
            if pid < 0 {
                let err = std::io::Error::last_os_error();
                libc::close(master);
                libc::close(slave);
                return Err(CoreError::Io(err));
            }

            if pid == 0 {
                // child
                let _ = libc::close(master);
                let _ = libc::setsid();
                let _ = libc::ioctl(slave, libc::TIOCSCTTY as _, 0);
                let _ = libc::dup2(slave, 0);
                let _ = libc::dup2(slave, 1);
                let _ = libc::dup2(slave, 2);
                if slave > 2 {
                    let _ = libc::close(slave);
                }

                // basic rlimits (M6/M8 foundation)
                let nproc = libc::rlimit {
                    rlim_cur: 256,
                    rlim_max: 256,
                };
                let _ = libc::setrlimit(libc::RLIMIT_NPROC, &nproc);
                let cpu = libc::rlimit {
                    rlim_cur: 300,
                    rlim_max: 300,
                };
                let _ = libc::setrlimit(libc::RLIMIT_CPU, &cpu);
                let fsize = libc::rlimit {
                    rlim_cur: 64 * 1024 * 1024,
                    rlim_max: 64 * 1024 * 1024,
                };
                let _ = libc::setrlimit(libc::RLIMIT_FSIZE, &fsize);

                let cwd_c = CString::new(config.cwd.as_os_str().as_bytes())
                    .unwrap_or_else(|_| CString::new("/").unwrap());
                let _ = libc::chdir(cwd_c.as_ptr());

                for (k, _) in std::env::vars() {
                    if let Ok(kc) = CString::new(k) {
                        let _ = libc::unsetenv(kc.as_ptr());
                    }
                }
                for (k, v) in &config.env {
                    if let (Ok(kc), Ok(vc)) = (CString::new(k.as_str()), CString::new(v.as_str())) {
                        let _ = libc::setenv(kc.as_ptr(), vc.as_ptr(), 1);
                    }
                }

                let shell_c = CString::new(config.shell_path.as_os_str().as_bytes())
                    .unwrap_or_else(|_| CString::new("/system/bin/sh").unwrap());
                let arg0 = CString::new("-sh").unwrap();
                let mut argv = [arg0.as_ptr(), std::ptr::null()];
                libc::execv(shell_c.as_ptr(), argv.as_mut_ptr());
                libc::_exit(127);
            }

            // parent
            let _ = libc::close(slave);
            let flags = libc::fcntl(master, libc::F_GETFL);
            if flags >= 0 {
                let _ = libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }

            Ok(PtySession {
                id: 0,
                master_fd: master,
                child_pid: pid,
                pgid: pid,
                closed: AtomicBool::new(false),
            })
        }
    }

    pub fn write(session: &PtySession, data: &[u8]) -> Result<()> {
        if session.closed.load(Ordering::SeqCst) {
            return Err(CoreError::InvalidArgument("pty closed".into()));
        }
        let mut off = 0;
        while off < data.len() {
            let n = unsafe {
                libc::write(
                    session.master_fd,
                    data[off..].as_ptr().cast(),
                    data.len() - off,
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                if err.kind() == std::io::ErrorKind::WouldBlock {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                return Err(CoreError::Io(err));
            }
            off += n as usize;
        }
        Ok(())
    }

    pub fn read_available(session: &PtySession) -> Result<Vec<u8>> {
        if session.closed.load(Ordering::SeqCst) {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = unsafe { libc::read(session.master_fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                out.extend_from_slice(&buf[..n as usize]);
                if out.len() >= 64 * 1024 {
                    break;
                }
                continue;
            }
            if n == 0 {
                break;
            }
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock
                || err.kind() == std::io::ErrorKind::Interrupted
            {
                break;
            }
            return Err(CoreError::Io(err));
        }
        Ok(out)
    }

    pub fn resize(session: &PtySession, cols: u16, rows: u16) -> Result<()> {
        if session.closed.load(Ordering::SeqCst) {
            return Err(CoreError::InvalidArgument("pty closed".into()));
        }
        let win = libc::winsize {
            ws_row: rows.max(1),
            ws_col: cols.max(1),
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let rc = unsafe { libc::ioctl(session.master_fd, libc::TIOCSWINSZ as _, &win) };
        if rc != 0 {
            return Err(CoreError::Io(std::io::Error::last_os_error()));
        }
        let _ = unsafe { libc::killpg(session.pgid, libc::SIGWINCH) };
        Ok(())
    }

    pub fn signal(session: &PtySession, sig: i32) -> Result<()> {
        if session.closed.load(Ordering::SeqCst) {
            return Err(CoreError::InvalidArgument("pty closed".into()));
        }
        let rc = unsafe { libc::killpg(session.pgid, sig) };
        if rc != 0 {
            return Err(CoreError::Io(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    pub fn try_wait(session: &PtySession) -> Result<Option<i32>> {
        let mut status: i32 = 0;
        let rc = unsafe { libc::waitpid(session.child_pid, &mut status as *mut _, libc::WNOHANG) };
        if rc == 0 {
            return Ok(None);
        }
        if rc < 0 {
            return Err(CoreError::Io(std::io::Error::last_os_error()));
        }
        let code = if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else if libc::WIFSIGNALED(status) {
            128 + libc::WTERMSIG(status)
        } else {
            -1
        };
        Ok(Some(code))
    }

    pub fn close(session: &PtySession) -> Result<()> {
        if session.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = unsafe { libc::killpg(session.pgid, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(Some(_)) = try_wait(session) {
                break;
            }
            if Instant::now() >= deadline {
                let _ = unsafe { libc::killpg(session.pgid, libc::SIGKILL) };
                let _ = try_wait(session);
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = unsafe { libc::close(session.master_fd) };
        Ok(())
    }
}

#[cfg(not(unix))]
mod unix_impl {
    use super::*;
    pub fn spawn(_config: &PtyConfig) -> Result<PtySession> {
        Err(CoreError::Internal(
            "PTY unsupported on this platform".into(),
        ))
    }
    pub fn write(_session: &PtySession, _data: &[u8]) -> Result<()> {
        Err(CoreError::Internal("PTY unsupported".into()))
    }
    pub fn read_available(_session: &PtySession) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }
    pub fn resize(_session: &PtySession, _cols: u16, _rows: u16) -> Result<()> {
        Err(CoreError::Internal("PTY unsupported".into()))
    }
    pub fn signal(_session: &PtySession, _sig: i32) -> Result<()> {
        Err(CoreError::Internal("PTY unsupported".into()))
    }
    pub fn try_wait(_session: &PtySession) -> Result<Option<i32>> {
        Ok(None)
    }
    pub fn close(_session: &PtySession) -> Result<()> {
        Ok(())
    }
}

pub use unix_impl::{close, read_available, resize, signal, spawn, try_wait, write};

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn spawn_read_and_close() {
        let cfg = PtyConfig {
            shell_path: PathBuf::from("/bin/sh"),
            cwd: std::env::current_dir().unwrap(),
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            cols: 80,
            rows: 24,
        };
        let session = spawn(&cfg).expect("spawn");
        write(&session, b"echo hi\nexit\n").expect("write");
        let mut out = Vec::new();
        for _ in 0..100 {
            let chunk = read_available(&session).expect("read");
            out.extend_from_slice(&chunk);
            if let Ok(Some(_)) = try_wait(&session) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("hi"), "output: {s}");
        close(&session).expect("close");
    }

    #[test]
    fn resize_and_signal_live_session() {
        let cfg = PtyConfig {
            shell_path: PathBuf::from("/bin/sh"),
            cwd: std::env::current_dir().unwrap(),
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            cols: 80,
            rows: 24,
        };
        let session = spawn(&cfg).expect("spawn");
        resize(&session, 100, 30).expect("resize");
        signal(&session, libc::SIGINT).expect("signal");
        write(&session, b"exit\n").expect("write exit");
        for _ in 0..50 {
            if let Ok(Some(_)) = try_wait(&session) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        close(&session).expect("close");
    }

    #[test]
    fn nonexistent_shell_returns_error() {
        let cfg = PtyConfig {
            shell_path: PathBuf::from("/no/such/shell"),
            cwd: std::env::current_dir().unwrap(),
            env: vec![],
            cols: 80,
            rows: 24,
        };
        assert!(spawn(&cfg).is_err());
    }
}
