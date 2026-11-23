use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};
use crate::proto::{LinuxDistro, LinuxEnvState};

#[derive(Clone, Debug)]
pub struct LinuxEnvStatus {
    pub state: LinuxEnvState,
    pub rootfs_path: String,
    pub distro: LinuxDistro,
    pub arch: String,
    pub size_bytes: u64,
    pub pkg_mgr: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct LinuxEnvManager {
    cache_root: PathBuf,
    state_path: PathBuf,
}

impl LinuxEnvManager {
    pub fn new(app_data_dir: &Path) -> Result<Self> {
        let cache_root = app_data_dir.join("ForgeBox").join("Cache").join("linux");
        let state_path = app_data_dir.join("internal").join("linux_env_state.txt");
        fs::create_dir_all(&cache_root)?;
        if let Some(p) = state_path.parent() {
            fs::create_dir_all(p)?;
        }
        Ok(Self {
            cache_root,
            state_path,
        })
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub fn status(&self) -> LinuxEnvStatus {
        let (state, distro, msg) = self.read_state();
        let rootfs = self.rootfs_path(distro);
        let size_bytes = dir_size(&rootfs).unwrap_or(0);
        let pkg_mgr = match distro {
            LinuxDistro::Alpine => "apk",
            LinuxDistro::Debian => "apt",
            _ => "",
        }
        .to_string();
        let ready = matches!(state, LinuxEnvState::LinuxEnvReady) && rootfs.is_dir();
        LinuxEnvStatus {
            state: if ready {
                LinuxEnvState::LinuxEnvReady
            } else if matches!(state, LinuxEnvState::LinuxEnvInstalling) {
                LinuxEnvState::LinuxEnvInstalling
            } else if matches!(state, LinuxEnvState::LinuxEnvFailed) {
                LinuxEnvState::LinuxEnvFailed
            } else {
                LinuxEnvState::LinuxEnvNotInstalled
            },
            rootfs_path: rootfs.to_string_lossy().into_owned(),
            distro,
            arch: std::env::consts::ARCH.to_string(),
            size_bytes,
            pkg_mgr,
            message: msg,
        }
    }

    pub fn mark_installing(&self, distro: LinuxDistro) -> Result<()> {
        self.write_state(LinuxEnvState::LinuxEnvInstalling, distro, "Installing")
    }
    pub fn mark_ready(&self, distro: LinuxDistro, msg: &str) -> Result<()> {
        let root = self.rootfs_path(distro);
        fs::create_dir_all(&root)?;
        self.write_state(LinuxEnvState::LinuxEnvReady, distro, msg)
    }
    pub fn mark_failed(&self, distro: LinuxDistro, msg: &str) -> Result<()> {
        self.write_state(LinuxEnvState::LinuxEnvFailed, distro, msg)
    }
    pub fn uninstall(&self) -> Result<LinuxEnvStatus> {
        let distro = self.read_state().1;
        if self.cache_root.exists() {
            fs::remove_dir_all(&self.cache_root)?;
        }
        fs::create_dir_all(&self.cache_root)?;
        self.write_state(
            LinuxEnvState::LinuxEnvNotInstalled,
            LinuxDistro::Unspecified,
            "Uninstalled",
        )?;
        let mut st = self.status();
        st.distro = distro;
        st.message = "Uninstalled".into();
        st.state = LinuxEnvState::LinuxEnvNotInstalled;
        st.size_bytes = 0;
        Ok(st)
    }
    pub fn rootfs_path(&self, distro: LinuxDistro) -> PathBuf {
        let name = match distro {
            LinuxDistro::Alpine => "alpine",
            LinuxDistro::Debian => "debian",
            _ => "unknown",
        };
        self.cache_root.join(name).join("rootfs")
    }
    pub fn ensure_guest_path_mapping(&self, project_dir: &Path) -> Result<String> {
        let canonical = fs::canonicalize(project_dir)?;
        Ok(canonical.to_string_lossy().into_owned())
    }

    fn read_state(&self) -> (LinuxEnvState, LinuxDistro, String) {
        let content = fs::read_to_string(&self.state_path).unwrap_or_default();
        let mut state = LinuxEnvState::LinuxEnvNotInstalled;
        let mut distro = LinuxDistro::Unspecified;
        let mut msg = String::new();
        for line in content.lines() {
            if let Some(v) = line.strip_prefix("state=") {
                state = match v {
                    "installing" => LinuxEnvState::LinuxEnvInstalling,
                    "ready" => LinuxEnvState::LinuxEnvReady,
                    "failed" => LinuxEnvState::LinuxEnvFailed,
                    _ => LinuxEnvState::LinuxEnvNotInstalled,
                };
            } else if let Some(v) = line.strip_prefix("distro=") {
                distro = match v {
                    "alpine" => LinuxDistro::Alpine,
                    "debian" => LinuxDistro::Debian,
                    _ => LinuxDistro::Unspecified,
                };
            } else if let Some(v) = line.strip_prefix("message=") {
                msg = v.to_string();
            }
        }
        (state, distro, msg)
    }
    fn write_state(&self, state: LinuxEnvState, distro: LinuxDistro, msg: &str) -> Result<()> {
        let state_s = match state {
            LinuxEnvState::LinuxEnvInstalling => "installing",
            LinuxEnvState::LinuxEnvReady => "ready",
            LinuxEnvState::LinuxEnvFailed => "failed",
            _ => "not_installed",
        };
        let distro_s = match distro {
            LinuxDistro::Alpine => "alpine",
            LinuxDistro::Debian => "debian",
            _ => "unknown",
        };
        let safe_msg = msg.replace('\n', " ").replace('\r', " ");
        fs::write(
            &self.state_path,
            format!("state={state_s}\ndistro={distro_s}\nmessage={safe_msg}\n"),
        )?;
        Ok(())
    }
}

pub fn dir_size(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let meta = fs::symlink_metadata(&p)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_file() {
            total = total.saturating_add(meta.len());
        } else if meta.is_dir() {
            for e in fs::read_dir(&p)? {
                stack.push(e?.path());
            }
        }
    }
    Ok(total)
}

pub fn validate_package_spec(spec: &str) -> Result<String> {
    let s = spec.trim();
    if s.is_empty() || s.len() > 200 {
        return Err(CoreError::InvalidArgument(
            "package spec must be 1-200 chars".into(),
        ));
    }
    if s.chars().any(|c| {
        !(c.is_ascii_alphanumeric() || matches!(c, '@' | '/' | '.' | '-' | '_' | ':' | '='))
    }) {
        return Err(CoreError::InvalidArgument(
            "package spec contains unsupported characters".into(),
        ));
    }
    Ok(s.to_string())
}

pub fn validate_binary_name(name: &str) -> Result<String> {
    let s = name.trim();
    if s.is_empty() || s.len() > 128 {
        return Err(CoreError::InvalidArgument(
            "binary name must be 1-128 chars".into(),
        ));
    }
    if s.chars()
        .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
    {
        return Err(CoreError::InvalidArgument(
            "binary name contains unsupported characters".into(),
        ));
    }
    Ok(s.to_string())
}

pub fn build_package_install_command(manager: &str, spec: &str) -> Result<String> {
    let spec = validate_package_spec(spec)?;
    Ok(match manager {
        "npm" => format!("npm install -g {spec}"),
        "apt" => format!("apt-get install -y {spec}"),
        "apk" => format!("apk add {spec}"),
        _ => {
            return Err(CoreError::InvalidArgument(
                "unsupported package manager".into(),
            ))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn status_defaults_not_installed_and_cache_root_under_forgebox_cache() {
        let t = tempdir().unwrap();
        let mgr = LinuxEnvManager::new(t.path()).unwrap();
        let st = mgr.status();
        assert_eq!(st.state, LinuxEnvState::LinuxEnvNotInstalled);
        assert!(st
            .rootfs_path
            .replace('\\', "/")
            .contains("/ForgeBox/Cache/linux/"));
    }

    #[test]
    fn install_state_transitions_and_uninstall() {
        let t = tempdir().unwrap();
        let mgr = LinuxEnvManager::new(t.path()).unwrap();
        mgr.mark_installing(LinuxDistro::Alpine).unwrap();
        assert_eq!(mgr.status().state, LinuxEnvState::LinuxEnvInstalling);
        mgr.mark_ready(LinuxDistro::Alpine, "Ready").unwrap();
        let st = mgr.status();
        assert_eq!(st.state, LinuxEnvState::LinuxEnvReady);
        assert_eq!(st.distro, LinuxDistro::Alpine);
        let un = mgr.uninstall().unwrap();
        assert_eq!(un.state, LinuxEnvState::LinuxEnvNotInstalled);
        assert!(!mgr.rootfs_path(LinuxDistro::Alpine).exists());
    }

    #[test]
    fn failed_state_recorded() {
        let t = tempdir().unwrap();
        let mgr = LinuxEnvManager::new(t.path()).unwrap();
        mgr.mark_failed(LinuxDistro::Debian, "boom").unwrap();
        let st = mgr.status();
        assert_eq!(st.state, LinuxEnvState::LinuxEnvFailed);
        assert!(st.message.contains("boom"));
    }

    #[test]
    fn package_spec_and_binary_validation() {
        assert!(validate_package_spec("@mmmbuto/codex-cli-termux").is_ok());
        assert!(validate_package_spec("bad spec;rm -rf").is_err());
        assert!(validate_binary_name("codex").is_ok());
        assert!(validate_binary_name("../codex").is_err());
    }

    #[test]
    fn package_install_command_is_shortcut_only() {
        assert_eq!(
            build_package_install_command("npm", "opencode-ai").unwrap(),
            "npm install -g opencode-ai"
        );
    }
}
