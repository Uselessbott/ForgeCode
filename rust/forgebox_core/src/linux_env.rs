use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::error::{CoreError, Result};
use crate::proto::{LinuxBootstrapMode, LinuxDistro, LinuxEnvState};

#[derive(Clone, Debug)]
pub struct LinuxEnvStatus {
    pub state: LinuxEnvState,
    pub rootfs_path: String,
    pub distro: LinuxDistro,
    pub arch: String,
    pub size_bytes: u64,
    pub pkg_mgr: String,
    pub message: String,
    pub rootfs_present: bool,
    pub proot_loader_present: bool,
    pub shell_binary: String,
}

#[derive(Clone, Debug)]
pub struct LinuxEnvManager {
    cache_root: PathBuf,
    state_path: PathBuf,
    native_lib_dir: PathBuf,
}

impl LinuxEnvManager {
    pub fn new(app_data_dir: &Path) -> Result<Self> {
        Self::new_with_native_lib_dir(app_data_dir, Path::new(""))
    }

    pub fn new_with_native_lib_dir(app_data_dir: &Path, native_lib_dir: &Path) -> Result<Self> {
        let cache_root = app_data_dir.join("ForgeBox").join("Cache").join("linux");
        let state_path = app_data_dir.join("internal").join("linux_env_state.txt");
        fs::create_dir_all(&cache_root)?;
        if let Some(p) = state_path.parent() {
            fs::create_dir_all(p)?;
        }
        Ok(Self {
            cache_root,
            state_path,
            native_lib_dir: native_lib_dir.to_path_buf(),
        })
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub fn status(&self) -> LinuxEnvStatus {
        let (state, distro, msg) = self.read_state();
        let rootfs = self.rootfs_path(distro);
        let rootfs_present = rootfs.is_dir() && rootfs.join("bin").is_dir();
        let proot_loader_present = self.proot_binary_path().is_file();
        let shell_binary = "/bin/sh".to_string();
        let size_bytes = dir_size(&self.cache_root).unwrap_or(0);
        let pkg_mgr = match distro {
            LinuxDistro::Alpine => "apk",
            LinuxDistro::Debian => "apt",
            _ => "",
        }
        .to_string();
        let ready =
            matches!(state, LinuxEnvState::LinuxEnvReady) && rootfs_present && proot_loader_present;
        let effective_state = if ready {
            LinuxEnvState::LinuxEnvReady
        } else if matches!(state, LinuxEnvState::LinuxEnvInstalling) {
            LinuxEnvState::LinuxEnvInstalling
        } else if matches!(state, LinuxEnvState::LinuxEnvFailed) {
            LinuxEnvState::LinuxEnvFailed
        } else {
            LinuxEnvState::LinuxEnvNotInstalled
        };
        LinuxEnvStatus {
            state: effective_state,
            rootfs_path: rootfs.to_string_lossy().into_owned(),
            distro,
            arch: guest_arch().to_string(),
            size_bytes,
            pkg_mgr,
            message: msg,
            rootfs_present,
            proot_loader_present,
            shell_binary,
        }
    }

    pub fn install_bundled_offline(
        &self,
        distro: LinuxDistro,
        source_tar_gz: &Path,
    ) -> Result<LinuxEnvStatus> {
        self.install_bundled_offline_with_progress(distro, source_tar_gz, &mut |_p, _m| {})
    }

    pub fn install_bundled_offline_with_progress<F: FnMut(i32, String)>(
        &self,
        distro: LinuxDistro,
        source_tar_gz: &Path,
        progress: &mut F,
    ) -> Result<LinuxEnvStatus> {
        if !matches!(distro, LinuxDistro::Alpine) {
            return Err(CoreError::InvalidArgument(
                "M10.1 bundled offline install supports Alpine only".into(),
            ));
        }
        self.mark_installing(distro, "Extracting bundled Alpine rootfs...")?;
        progress(5, "Extracting bundled Alpine rootfs...".into());
        let rootfs = self.rootfs_path(distro);
        let staging = self.cache_root.join(format!(
            ".extract-{}-{}",
            distro_name(distro),
            std::process::id()
        ));
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        fs::create_dir_all(&staging)?;
        progress(10, "Extracting rootfs archive...".into());
        let extract_res = extract_tar_gz_safe_with_progress(source_tar_gz, &staging, &mut |done, _total, msg| {
            let pct = (10 + (done / 50).min(65) as i32).min(75);
            progress(pct, msg);
        });
        if let Err(e) = extract_res {
            let _ = fs::remove_dir_all(&staging);
            let msg = format!("Extraction failed: {e}");
            self.mark_failed(distro, &msg)?;
            return Err(CoreError::InvalidArgument(msg));
        }
        progress(80, "Finalizing rootfs...".into());
        if rootfs.exists() {
            fs::remove_dir_all(&rootfs)?;
        }
        if let Some(parent) = rootfs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&staging, &rootfs).or_else(|_| {
            copy_dir_recursive(&staging, &rootfs)?;
            let _ = fs::remove_dir_all(&staging);
            Ok::<(), CoreError>(())
        })?;
        progress(90, "Configuring PRoot wrapper...".into());
        self.ensure_proot_wrapper()?;
        if !self.proot_binary_path().is_file() {
            let msg = "PRoot loader not present in native lib dir".to_string();
            self.mark_failed(distro, &msg)?;
            return Err(CoreError::InvalidArgument(msg));
        }
        if !rootfs.join("bin/sh").exists() && !rootfs.join("bin/busybox").exists() {
            let msg = "Extracted rootfs missing /bin/sh".to_string();
            self.mark_failed(distro, &msg)?;
            return Err(CoreError::InvalidArgument(msg));
        }
        self.write_state(
            LinuxEnvState::LinuxEnvReady,
            distro,
            "Ready (offline bundled Alpine)",
        )?;
        progress(100, "Ready (offline bundled Alpine)".into());
        Ok(self.status())
    }

    pub fn mark_installing(&self, distro: LinuxDistro, msg: &str) -> Result<()> {
        self.write_state(LinuxEnvState::LinuxEnvInstalling, distro, msg)
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
        self.cache_root.join(distro_name(distro)).join("rootfs")
    }
    pub fn proot_binary_path(&self) -> PathBuf {
        self.cache_root.join("bin").join("proot")
    }
    pub fn proot_native_path(&self) -> PathBuf {
        self.native_lib_dir.join("libproot.so")
    }
    pub fn proot_loader_path(&self) -> PathBuf {
        self.native_lib_dir.join("libprootloader.so")
    }
    pub fn proot_loader32_path(&self) -> PathBuf {
        self.native_lib_dir.join("libprootloader32.so")
    }
    pub fn ensure_proot_wrapper(&self) -> Result<PathBuf> {
        let bin_dir = self.cache_root.join("bin");
        fs::create_dir_all(&bin_dir)?;
        let wrapper = bin_dir.join("proot");
        let native = self.proot_native_path();
        let loader = self.proot_loader_path();
        let loader32 = self.proot_loader32_path();
        let tmp_dir = self.cache_root.join("tmp");
        fs::create_dir_all(&tmp_dir)?;
        let content = format!(
            "#!/system/bin/sh\nexport PROOT_LOADER=\"{}\"\nexport PROOT_LOADER_32=\"{}\"\nexport PROOT_TMP_DIR=\"{}\"\nexec \"{}\" \"$@\"\n",
            loader.to_string_lossy(),
            loader32.to_string_lossy(),
            tmp_dir.to_string_lossy(),
            native.to_string_lossy()
        );
        fs::write(&wrapper, content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))?;
        }
        Ok(wrapper)
    }

    pub fn build_proot_argv(
        &self,
        rootfs: &Path,
        project_dir: &Path,
        guest_cwd: &str,
    ) -> Vec<String> {
        let bind_dev = "/dev";
        let bind_proc = "/proc";
        let bind_sys = "/sys";
        let tmp = self.cache_root.join("tmp");
        let proj = project_dir.to_string_lossy().into_owned();
        vec![
            self.proot_binary_path().to_string_lossy().into_owned(),
            "--link2symlink".into(),
            "-0".into(),
            "-r".into(),
            rootfs.to_string_lossy().into_owned(),
            "-b".into(),
            bind_dev.into(),
            "-b".into(),
            bind_proc.into(),
            "-b".into(),
            bind_sys.into(),
            "-b".into(),
            format!("{proj}:{proj}"),
            "-b".into(),
            format!("{}:/tmp", tmp.to_string_lossy()),
            "-w".into(),
            guest_cwd.to_string(),
            "/bin/sh".into(),
            "-i".into(),
        ]
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
                    "installing" | "extracting" => LinuxEnvState::LinuxEnvInstalling,
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
        let distro_s = distro_name(distro);
        let safe_msg = msg.replace('\n', " ").replace('\r', " ");
        fs::write(
            &self.state_path,
            format!("state={state_s}\ndistro={distro_s}\nmessage={safe_msg}\n"),
        )?;
        Ok(())
    }
}

pub fn distro_name(distro: LinuxDistro) -> &'static str {
    match distro {
        LinuxDistro::Alpine => "alpine",
        LinuxDistro::Debian => "debian",
        _ => "unknown",
    }
}

pub fn guest_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        other => other,
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

pub fn extract_tar_gz_safe(source: &Path, dest: &Path) -> Result<()> {
    extract_tar_gz_safe_with_progress(source, dest, &mut |_done, _total, _msg| {})
}

pub fn extract_tar_gz_safe_with_progress<F: FnMut(u64, u64, String)>(
    source: &Path,
    dest: &Path,
    progress: &mut F,
) -> Result<()> {
    let file = fs::File::open(source)?;
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    fs::create_dir_all(dest)?;
    let dest_canon = fs::canonicalize(dest)?;
    let mut done_entries: u64 = 0;
    for entry in archive.entries()? {
        done_entries += 1;
        if done_entries % 25 == 0 {
            progress(done_entries, total, format!("Extracting rootfs... ({done_entries} entries)"));
        }
        let mut entry = entry?;
        let entry_type = entry.header().entry_type();
        let rel = entry.path()?.into_owned();
        let safe_rel = sanitize_archive_rel_path(&rel)?;
        if safe_rel.as_os_str().is_empty() {
            continue;
        }
        let out_path = dest_canon.join(&safe_rel);
        if !out_path.starts_with(&dest_canon) {
            return Err(CoreError::InvalidArgument(
                "archive traversal rejected".into(),
            ));
        }
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            // Alpine minirootfs contains symlinks; allow only if target stays within rootfs.
            let link_target = entry
                .link_name()?
                .ok_or_else(|| CoreError::InvalidArgument("symlink missing target".into()))?
                .into_owned();
            let resolved = resolve_archive_link_target(&dest_canon, &out_path, &link_target)?;
            if !resolved.starts_with(&dest_canon) {
                return Err(CoreError::InvalidArgument(
                    "archive symlink escapes rootfs".into(),
                ));
            }
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if out_path.exists() || fs::symlink_metadata(&out_path).is_ok() {
                let _ = fs::remove_file(&out_path);
            }
            #[cfg(unix)]
            {
                if entry_type.is_symlink() {
                    std::os::unix::fs::symlink(&link_target, &out_path)?;
                } else {
                    fs::hard_link(resolved, &out_path)?;
                }
            }
            #[cfg(not(unix))]
            {
                // Windows host tests: represent symlink as a small text file marker? No, better skip unsafe semantics.
                // For M10.1 host tests we only need extraction validation; keep a best-effort copy for file hardlinks.
                if entry_type.is_hard_link() {
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::copy(resolved, &out_path)?;
                } else {
                    // Write a marker file containing target; guest Linux will use real symlink on device.
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&out_path, link_target.to_string_lossy().as_bytes())?;
                }
            }
            continue;
        }
        if entry_type.is_dir() {
            fs::create_dir_all(&out_path)?;
            continue;
        }
        if entry_type.is_file() {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = fs::File::create(&out_path)?;
            io::copy(&mut entry, &mut out)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(mode) = entry.header().mode() {
                    fs::set_permissions(&out_path, fs::Permissions::from_mode(mode))?;
                }
            }
            continue;
        }
        return Err(CoreError::InvalidArgument(format!(
            "unsupported archive entry type: {:?}",
            entry_type
        )));
    }
    Ok(())
}

fn sanitize_archive_rel_path(path: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(s) => out.push(s),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(CoreError::InvalidArgument(
                    "archive path traversal rejected".into(),
                ))
            }
        }
    }
    Ok(out)
}

fn resolve_archive_link_target(root: &Path, link_path: &Path, target: &Path) -> Result<PathBuf> {
    let base = link_path.parent().unwrap_or(root);
    let mut resolved = if target.is_absolute() {
        root.to_path_buf()
    } else {
        base.to_path_buf()
    };
    for comp in target.components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(s) => resolved.push(s),
            Component::ParentDir => {
                if !resolved.pop() || !resolved.starts_with(root) {
                    return Err(CoreError::InvalidArgument(
                        "archive symlink escapes rootfs".into(),
                    ));
                }
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Ok(resolved)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let ty = e.file_type()?;
        let to = dst.join(e.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&e.path(), &to)?;
        } else if ty.is_file() {
            fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tempfile::tempdir;

    fn make_tar_gz(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut ar = tar::Builder::new(Vec::new());
        for (name, data, mode) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_path(name).unwrap();
            h.set_size(data.len() as u64);
            h.set_mode(*mode);
            h.set_cksum();
            ar.append(&h, *data).unwrap();
        }
        let tar_bytes = ar.into_inner().unwrap();
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        io::copy(&mut &tar_bytes[..], &mut enc).unwrap();
        enc.finish().unwrap()
    }

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
        assert!(!st.rootfs_present);
        assert_eq!(st.shell_binary, "/bin/sh");
    }

    #[test]
    fn install_bundled_offline_extracts_and_marks_ready() {
        let t = tempdir().unwrap();
        let native = t.path().join("native");
        fs::create_dir_all(&native).unwrap();
        fs::write(native.join("libproot.so"), b"fake-proot").unwrap();
        fs::write(native.join("libprootloader.so"), b"loader").unwrap();
        fs::write(native.join("libprootloader32.so"), b"loader32").unwrap();
        let mgr = LinuxEnvManager::new_with_native_lib_dir(t.path(), &native).unwrap();
        let tar_gz = make_tar_gz(&[
            ("bin/sh", b"#!/bin/sh\n", 0o755),
            ("etc/os-release", b"alpine", 0o644),
        ]);
        let src = t.path().join("alpine.tar.gz");
        fs::write(&src, tar_gz).unwrap();
        let st = mgr
            .install_bundled_offline(LinuxDistro::Alpine, &src)
            .unwrap();
        assert_eq!(st.state, LinuxEnvState::LinuxEnvReady);
        assert!(st.rootfs_present);
        assert!(st.proot_loader_present);
        assert!(mgr.rootfs_path(LinuxDistro::Alpine).join("bin/sh").exists());
        assert!(mgr.proot_binary_path().exists());
    }

    #[test]
    fn extract_rejects_traversal_and_absolute_paths() {
        let t = tempdir().unwrap();
        let mut ar = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_size(3);
        h.set_mode(0o644);
        h.set_cksum();
        let name = b"/abs.txt\0";
        h.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
        ar.append(&h, &b"bad"[..]).unwrap();
        let tar_bytes = ar.into_inner().unwrap();
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        io::copy(&mut &tar_bytes[..], &mut enc).unwrap();
        let src = t.path().join("bad.tar.gz");
        fs::write(&src, enc.finish().unwrap()).unwrap();
        let dest = t.path().join("dest");
        let err = extract_tar_gz_safe(&src, &dest);
        assert!(err.is_err());
    }

    #[test]
    fn uninstall_removes_cache() {
        let t = tempdir().unwrap();
        let mgr = LinuxEnvManager::new(t.path()).unwrap();
        fs::create_dir_all(mgr.rootfs_path(LinuxDistro::Alpine)).unwrap();
        mgr.mark_ready(LinuxDistro::Alpine, "ready").unwrap();
        let st = mgr.uninstall().unwrap();
        assert_eq!(st.state, LinuxEnvState::LinuxEnvNotInstalled);
        assert!(!mgr.rootfs_path(LinuxDistro::Alpine).exists());
    }

    #[test]
    fn proot_argv_has_required_binds_and_cwd() {
        let t = tempdir().unwrap();
        let mgr = LinuxEnvManager::new(t.path()).unwrap();
        let rootfs = t.path().join("rootfs");
        let proj = t.path().join("proj");
        fs::create_dir_all(&rootfs).unwrap();
        fs::create_dir_all(&proj).unwrap();
        let argv = mgr.build_proot_argv(&rootfs, &proj, "/work");
        let joined = argv.join(" ");
        assert!(joined.contains("--link2symlink"));
        assert!(joined.contains("-0"));
        assert!(joined.contains(&format!("-r {}", rootfs.to_string_lossy())));
        assert!(joined.contains("-b /dev"));
        assert!(joined.contains("-b /proc"));
        assert!(joined.contains("-b /sys"));
        assert!(joined.contains(&format!(
            "{}:{}",
            proj.to_string_lossy(),
            proj.to_string_lossy()
        )));
        assert!(joined.contains("-w /work"));
        assert!(joined.ends_with("/bin/sh -i"));
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

    #[test]
    fn proot_argv_exact_offline_shape() {
        let t = tempdir().unwrap();
        let mgr = LinuxEnvManager::new(t.path()).unwrap();
        let rootfs = t.path().join("ForgeBox/Cache/linux/alpine/rootfs");
        let proj = t.path().join("ForgeBox/Projects/p1");
        fs::create_dir_all(&rootfs).unwrap();
        fs::create_dir_all(&proj).unwrap();
        let argv = mgr.build_proot_argv(&rootfs, &proj, &proj.to_string_lossy());
        assert_eq!(argv[1], "--link2symlink");
        assert_eq!(argv[2], "-0");
        assert_eq!(argv[3], "-r");
        assert_eq!(argv[4], rootfs.to_string_lossy());
        assert!(argv.windows(2).any(|w| w[0] == "-b" && w[1] == "/dev"));
        assert!(argv.windows(2).any(|w| w[0] == "-b" && w[1] == "/proc"));
        assert!(argv.windows(2).any(|w| w[0] == "-b" && w[1] == "/sys"));
        let proj_bind = format!("{}:{}", proj.to_string_lossy(), proj.to_string_lossy());
        assert!(argv.windows(2).any(|w| w[0] == "-b" && w[1] == proj_bind));
        let tmp_bind = format!("{}:/tmp", mgr.cache_root().join("tmp").to_string_lossy());
        assert!(argv.windows(2).any(|w| w[0] == "-b" && w[1] == tmp_bind));
        let widx = argv.iter().position(|x| x == "-w").unwrap();
        assert_eq!(argv[widx + 1], proj.to_string_lossy());
        assert_eq!(argv[argv.len() - 2], "/bin/sh");
        assert_eq!(argv[argv.len() - 1], "-i");
    }
}
