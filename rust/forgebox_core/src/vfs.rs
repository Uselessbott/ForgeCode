use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::{CoreError, Result};

/// Project names become directory names, so keep them deliberately conservative.
pub fn validate_project_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(CoreError::InvalidArgument(
            "project name must be 1-64 characters".into(),
        ));
    }
    if name == "." || name == ".." {
        return Err(CoreError::InvalidArgument(
            "project name cannot be . or ..".into(),
        ));
    }
    if name
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Err(CoreError::InvalidArgument(
            "project name may contain only letters, digits, '-', '_' and '.'".into(),
        ));
    }
    Ok(())
}

/// Validate a project-relative path. Backslashes are rejected even on Unix so
/// the same rules hold when a project is later opened on a different platform.
pub fn validate_relative_path(raw: &str, allow_empty: bool) -> Result<PathBuf> {
    if raw.contains('\0') || raw.contains('\\') {
        return Err(CoreError::InvalidArgument(
            "path contains a forbidden character".into(),
        ));
    }

    let mut out = PathBuf::new();
    for component in Path::new(raw).components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment.to_string_lossy();
                if segment.is_empty()
                    || segment == "."
                    || segment == ".."
                    || segment.chars().any(|c| c.is_control())
                {
                    return Err(CoreError::InvalidArgument(
                        "path contains an invalid segment".into(),
                    ));
                }
                out.push(segment.as_ref());
            }
            _ => {
                return Err(CoreError::InvalidArgument(
                    "path must be project-relative".into(),
                ));
            }
        }
    }

    if out.as_os_str().is_empty() && !allow_empty {
        return Err(CoreError::InvalidArgument("path cannot be empty".into()));
    }
    Ok(out)
}

pub fn canonical_root(root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(root)?;
    fs::canonicalize(root).map_err(Into::into)
}

/// Resolve a validated relative path under `root`, rejecting symlinks in every
/// existing component and refusing to leave the canonical root.
pub fn resolve(root: &Path, relative: &Path, must_exist: bool) -> Result<PathBuf> {
    let root = canonical_root(root)?;
    let mut cursor = root.clone();

    for component in relative.components() {
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CoreError::PathEscape(format!(
                    "symlink rejected: {}",
                    cursor.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if must_exist {
                    return Err(CoreError::NotFound(format!(
                        "path not found: {}",
                        cursor.display()
                    )));
                }
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }

    if must_exist {
        let canonical = fs::canonicalize(&cursor)?;
        if !canonical.starts_with(&root) {
            return Err(CoreError::PathEscape(format!(
                "path left project root: {}",
                canonical.display()
            )));
        }
        Ok(canonical)
    } else {
        Ok(cursor)
    }
}

/// Resolve the final component of a file that is about to be written. Parent
/// directories are created; existing final symlinks are rejected.
pub fn resolve_for_write(root: &Path, relative: &Path) -> Result<PathBuf> {
    let file_name = relative
        .file_name()
        .ok_or_else(|| CoreError::InvalidArgument("path has no file name".into()))?;
    let parent_relative = relative.parent().unwrap_or_else(|| Path::new(""));
    let parent = resolve(root, parent_relative, false)?;
    fs::create_dir_all(&parent)?;
    let parent = resolve(root, parent_relative, true)?;

    let final_path = parent.join(file_name);
    if let Ok(metadata) = fs::symlink_metadata(&final_path) {
        if metadata.file_type().is_symlink() {
            return Err(CoreError::PathEscape(format!(
                "symlink rejected: {}",
                final_path.display()
            )));
        }
    }
    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_relative_paths() {
        let path = validate_relative_path("src/main.rs", false).unwrap();
        assert_eq!(path, PathBuf::from("src/main.rs"));
        assert!(validate_relative_path("", true)
            .unwrap()
            .as_os_str()
            .is_empty());
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        assert!(validate_relative_path("../secret", false).is_err());
        assert!(validate_relative_path("src/../../secret", false).is_err());
        assert!(validate_relative_path("/etc/passwd", false).is_err());
        assert!(validate_relative_path("a\\b", false).is_err());
        assert!(validate_relative_path("a\0b", false).is_err());
        assert!(validate_relative_path("", false).is_err());
    }

    #[test]
    fn rejects_unsafe_project_names() {
        validate_project_name("MyPythonApp").unwrap();
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name("..").is_err());
        assert!(validate_project_name("a/b").is_err());
        assert!(validate_project_name("a b").is_err());
    }

    #[test]
    fn resolve_rejects_symlink_escape() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects").join("p1");
        fs::create_dir_all(&root).unwrap();

        let outside = temp.path().join("outside");
        fs::write(&outside, b"secret").unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("leak")).unwrap();
            assert!(resolve(&root, Path::new("leak"), true).is_err());
        }

        let inside = root.join("safe.txt");
        fs::write(&inside, b"ok").unwrap();
        let resolved = resolve(&root, Path::new("safe.txt"), true).unwrap();
        assert_eq!(resolved, fs::canonicalize(&inside).unwrap());
    }
}
