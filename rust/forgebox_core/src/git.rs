use std::path::{Path, PathBuf};

use git2::{build::RepoBuilder, BranchType, DiffOptions, Repository, Signature, StatusOptions};

use crate::error::{CoreError, Result};
use crate::vfs;

pub struct GitStatusEntry {
    pub path: String,
    pub status: String,
    pub is_binary: bool,
}
pub struct GitStatusResult {
    pub staged: Vec<GitStatusEntry>,
    pub unstaged: Vec<GitStatusEntry>,
    pub untracked: Vec<GitStatusEntry>,
    pub branch: String,
}
pub struct GitCommitInfo {
    pub commit_id: String,
    pub author: String,
    pub email: String,
    pub message: String,
    pub timestamp: i64,
}

fn repo_for(project_root: &Path) -> Result<Repository> {
    Repository::open(project_root).map_err(|e| CoreError::InvalidArgument(format!("git open: {e}")))
}

pub fn init_repo(project_root: &Path) -> Result<()> {
    let repo = Repository::init(project_root)
        .map_err(|e| CoreError::InvalidArgument(format!("git init: {e}")))?;
    set_hooks_path_empty(project_root)?;
    if let Ok(mut cfg) = repo.config() {
        let _ = cfg.set_str(
            "core.hooksPath",
            &empty_hooks_dir(project_root).to_string_lossy(),
        );
    }
    Ok(())
}

pub fn empty_hooks_dir(project_root: &Path) -> PathBuf {
    project_root.join(".forgebox-hooks-empty")
}

pub fn set_hooks_path_empty(project_root: &Path) -> Result<()> {
    let dir = empty_hooks_dir(project_root);
    std::fs::create_dir_all(&dir)?;
    let repo = repo_for(project_root)?;
    let mut cfg = repo
        .config()
        .map_err(|e| CoreError::InvalidArgument(format!("git config: {e}")))?;
    cfg.set_str("core.hooksPath", &dir.to_string_lossy())
        .map_err(|e| CoreError::InvalidArgument(format!("git config: {e}")))?;
    Ok(())
}

pub fn status(project_root: &Path) -> Result<GitStatusResult> {
    let repo = repo_for(project_root)?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true);
    let statuses = repo
        .statuses(Some(&mut opts))
        .map_err(|e| CoreError::InvalidArgument(format!("git status: {e}")))?;
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut untracked = Vec::new();
    for e in statuses.iter() {
        let path = e.path().unwrap_or("").to_string();
        let s = e.status();
        let is_binary = e
            .head_to_index()
            .and_then(|d| d.new_file().is_binary().then_some(true))
            .or_else(|| {
                e.index_to_workdir()
                    .and_then(|d| d.new_file().is_binary().then_some(true))
            })
            .unwrap_or(false);
        if s.is_index_new()
            || s.is_index_modified()
            || s.is_index_deleted()
            || s.is_index_renamed()
            || s.is_index_typechange()
        {
            let code = if s.is_index_new() {
                "A"
            } else if s.is_index_modified() {
                "M"
            } else if s.is_index_deleted() {
                "D"
            } else if s.is_index_renamed() {
                "R"
            } else {
                "T"
            };
            staged.push(GitStatusEntry {
                path: path.clone(),
                status: code.into(),
                is_binary,
            });
        }
        if s.is_wt_modified() || s.is_wt_deleted() || s.is_wt_typechange() || s.is_wt_renamed() {
            let code = if s.is_wt_modified() {
                "M"
            } else if s.is_wt_deleted() {
                "D"
            } else if s.is_wt_renamed() {
                "R"
            } else {
                "T"
            };
            unstaged.push(GitStatusEntry {
                path: path.clone(),
                status: code.into(),
                is_binary,
            });
        }
        if s.is_wt_new() {
            untracked.push(GitStatusEntry {
                path,
                status: "?".into(),
                is_binary,
            });
        }
    }
    let branch = repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().map(|s| s.to_string()))
        .unwrap_or_else(|| "HEAD".into());
    Ok(GitStatusResult {
        staged,
        unstaged,
        untracked,
        branch,
    })
}

pub fn diff(project_root: &Path, scope: &str, path: Option<&str>) -> Result<String> {
    let repo = repo_for(project_root)?;
    let mut opts = DiffOptions::new();
    if let Some(p) = path {
        if !p.is_empty() {
            opts.pathspec(p);
        }
    }
    let diff = if scope == "staged" {
        let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        repo.diff_tree_to_index(head.as_ref(), None, Some(&mut opts))
    } else {
        repo.diff_index_to_workdir(None, Some(&mut opts))
    }
    .map_err(|e| CoreError::InvalidArgument(format!("git diff: {e}")))?;
    let mut out = String::new();
    diff.print(git2::DiffFormat::Patch, |_d, _h, l| {
        let c = l.origin() as char;
        if matches!(c, '+' | '-' | ' ' | '@' | 'H' | 'F' | 'I') {
            out.push(c);
        }
        out.push_str(std::str::from_utf8(l.content()).unwrap_or(""));
        true
    })
    .map_err(|e| CoreError::InvalidArgument(format!("git diff print: {e}")))?;
    Ok(out)
}

pub fn stage(project_root: &Path, files: &[String]) -> Result<()> {
    let repo = repo_for(project_root)?;
    let mut index = repo
        .index()
        .map_err(|e| CoreError::InvalidArgument(format!("git index: {e}")))?;
    for f in files {
        let rel = vfs::validate_relative_path(f, false)?;
        let abs = project_root.join(&rel);
        if abs.exists() {
            index
                .add_path(&rel)
                .map_err(|e| CoreError::InvalidArgument(format!("git add: {e}")))?;
        } else {
            index
                .remove_path(&rel)
                .map_err(|e| CoreError::InvalidArgument(format!("git remove: {e}")))?;
        }
    }
    index
        .write()
        .map_err(|e| CoreError::InvalidArgument(format!("git index write: {e}")))?;
    Ok(())
}

pub fn unstage(project_root: &Path, files: &[String]) -> Result<()> {
    let repo = repo_for(project_root)?;
    let head = repo.head().ok().and_then(|h| h.target());
    for f in files {
        let rel = vfs::validate_relative_path(f, false)?;
        let path = rel.to_string_lossy().to_string();
        if let Some(oid) = head {
            let obj = repo
                .find_object(oid, None)
                .map_err(|e| CoreError::InvalidArgument(format!("git reset: {e}")))?;
            let _ = repo.reset_default(Some(&obj), [path.as_str()]);
        } else {
            let mut index = repo
                .index()
                .map_err(|e| CoreError::InvalidArgument(format!("git index: {e}")))?;
            let _ = index.remove_path(&rel);
            index
                .write()
                .map_err(|e| CoreError::InvalidArgument(format!("git index write: {e}")))?;
        }
    }
    Ok(())
}

pub fn commit(
    project_root: &Path,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> Result<String> {
    if message.trim().is_empty() {
        return Err(CoreError::InvalidArgument(
            "commit message cannot be empty".into(),
        ));
    }
    let repo = repo_for(project_root)?;
    let mut index = repo
        .index()
        .map_err(|e| CoreError::InvalidArgument(format!("git index: {e}")))?;
    if index.is_empty() && repo.head().is_err() {
        return Err(CoreError::InvalidArgument("no staged changes".into()));
    }
    let tree_id = index
        .write_tree()
        .map_err(|e| CoreError::InvalidArgument(format!("git write tree: {e}")))?;
    let tree = repo
        .find_tree(tree_id)
        .map_err(|e| CoreError::InvalidArgument(format!("git tree: {e}")))?;
    let sig_name = if author_name.trim().is_empty() { "ForgeBox" } else { author_name.trim() };
    let sig_email = if author_email.trim().is_empty() { "forgebox@local" } else { author_email.trim() };
    let sig = Signature::now(sig_name, sig_email)
        .map_err(|e| CoreError::InvalidArgument(format!("git signature: {e}")))?;
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    if let Some(p) = parent.as_ref() {
        if p.tree_id() == tree_id {
            return Err(CoreError::InvalidArgument("no staged changes".into()));
        }
    }
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let update_ref = match repo.head() {
        Ok(head) if head.is_branch() => Some(head.name().unwrap_or("HEAD").to_string()),
        _ => Some("HEAD".to_string()),
    };
    let oid = repo
        .commit(update_ref.as_deref(), &sig, &sig, message, &tree, &parents)
        .map_err(|e| CoreError::InvalidArgument(format!("git commit: {e}")))?;
    if repo.head().ok().and_then(|h| h.target()).as_ref() != Some(&oid) {
        // Ensure HEAD resolves to the new commit (unborn HEAD / initial commit case).
        let head_ref = repo.head().ok();
        let target_ref = head_ref
            .as_ref()
            .and_then(|h| h.name())
            .unwrap_or("refs/heads/master");
        repo.reference(target_ref, oid, true, "forgebox commit")
            .map_err(|e| CoreError::InvalidArgument(format!("git commit ref update: {e}")))?;
        repo.set_head(target_ref)
            .map_err(|e| CoreError::InvalidArgument(format!("git commit set head: {e}")))?;
    }
    Ok(oid.to_string())
}

pub fn branch_list(project_root: &Path) -> Result<Vec<(String, bool)>> {
    let repo = repo_for(project_root)?;
    let branches = repo
        .branches(Some(BranchType::Local))
        .map_err(|e| CoreError::InvalidArgument(format!("git branches: {e}")))?;
    let mut out = Vec::new();
    for b in branches {
        let (b, _) = b.map_err(|e| CoreError::InvalidArgument(format!("git branch: {e}")))?;
        let name = b
            .name()
            .map_err(|e| CoreError::InvalidArgument(format!("git branch name: {e}")))?
            .unwrap_or("")
            .to_string();
        out.push((name, b.is_head()));
    }
    Ok(out)
}

pub fn branch_create(project_root: &Path, name: &str) -> Result<()> {
    let repo = repo_for(project_root)?;
    let head = repo
        .head()
        .map_err(|e| CoreError::InvalidArgument(format!("git head: {e}")))?
        .peel_to_commit()
        .map_err(|e| CoreError::InvalidArgument(format!("git commit: {e}")))?;
    repo.branch(name, &head, false)
        .map_err(|e| CoreError::InvalidArgument(format!("git branch create: {e}")))?;
    Ok(())
}

pub fn branch_switch(project_root: &Path, name: &str) -> Result<()> {
    let repo = repo_for(project_root)?;
    let obj = repo
        .revparse_single(&format!("refs/heads/{name}"))
        .map_err(|e| CoreError::InvalidArgument(format!("git switch: {e}")))?;
    repo.checkout_tree(&obj, None)
        .map_err(|e| CoreError::InvalidArgument(format!("git checkout: {e}")))?;
    repo.set_head(&format!("refs/heads/{name}"))
        .map_err(|e| CoreError::InvalidArgument(format!("git set head: {e}")))?;
    Ok(())
}

pub fn log(project_root: &Path, limit: u32) -> Result<Vec<GitCommitInfo>> {
    let repo = repo_for(project_root)?;
    let mut revwalk = repo
        .revwalk()
        .map_err(|e| CoreError::InvalidArgument(format!("git log: {e}")))?;
    if revwalk.push_head().is_err() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for oid in revwalk.take(limit.max(1) as usize) {
        let oid = oid.map_err(|e| CoreError::InvalidArgument(format!("git oid: {e}")))?;
        let c = repo
            .find_commit(oid)
            .map_err(|e| CoreError::InvalidArgument(format!("git commit: {e}")))?;
        let a = c.author();
        out.push(GitCommitInfo {
            commit_id: oid.to_string(),
            author: a.name().unwrap_or("").to_string(),
            email: a.email().unwrap_or("").to_string(),
            message: c.message().unwrap_or("").to_string(),
            timestamp: c.time().seconds(),
        });
    }
    Ok(out)
}

pub fn show(project_root: &Path, commit_id: &str) -> Result<String> {
    let repo = repo_for(project_root)?;
    let oid = git2::Oid::from_str(commit_id)
        .map_err(|e| CoreError::InvalidArgument(format!("git oid: {e}")))?;
    let commit = repo
        .find_commit(oid)
        .map_err(|e| CoreError::InvalidArgument(format!("git show: {e}")))?;
    let tree = commit
        .tree()
        .map_err(|e| CoreError::InvalidArgument(format!("git tree: {e}")))?;
    let parent_tree = if commit.parent_count() > 0 {
        Some(
            commit
                .parent(0)
                .map_err(|e| CoreError::InvalidArgument(format!("git parent: {e}")))?
                .tree()
                .map_err(|e| CoreError::InvalidArgument(format!("git parent tree: {e}")))?,
        )
    } else {
        None
    };
    let diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
        .map_err(|e| CoreError::InvalidArgument(format!("git diff: {e}")))?;
    let mut out = String::new();
    diff.print(git2::DiffFormat::Patch, |_d, _h, l| {
        let c = l.origin() as char;
        if matches!(c, '+' | '-' | ' ' | '@' | 'H' | 'F' | 'I') {
            out.push(c);
        }
        out.push_str(std::str::from_utf8(l.content()).unwrap_or(""));
        true
    })
    .map_err(|e| CoreError::InvalidArgument(format!("git diff: {e}")))?;
    Ok(out)
}

pub fn clone_local(source_root: &Path, target_root: &Path) -> Result<()> {
    if target_root.exists() {
        return Err(CoreError::InvalidArgument("target exists".into()));
    }
    if let Some(parent) = target_root.parent() {
        std::fs::create_dir_all(parent)?;
    }
    RepoBuilder::new()
        .clone(&source_root.to_string_lossy(), target_root)
        .map_err(|e| CoreError::InvalidArgument(format!("git clone local: {e}")))?;
    Ok(())
}

pub fn is_destructive_git_command(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    lower.contains("push --force")
        || lower.contains("push -f")
        || lower.contains("--force-with-lease")
        || lower.contains("push --mirror")
        || lower.contains("reset --hard")
        || lower.contains("filter-branch")
        || lower.contains("filter-repo")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn init_repo_with_commit(root: &Path) {
        init_repo(root).unwrap();
        fs::write(root.join("a.txt"), "hello").unwrap();
        stage(root, &["a.txt".to_string()]).unwrap();
        commit(root, "init", "Test", "t@test").unwrap();
    }

    #[test]
    fn status_diff_stage_commit_round_trip() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        init_repo(root).unwrap();
        fs::write(root.join("a.txt"), "hello").unwrap();
        let s = status(root).unwrap();
        assert!(s.untracked.iter().any(|e| e.path == "a.txt"));
        stage(root, &["a.txt".to_string()]).unwrap();
        let s2 = status(root).unwrap();
        assert!(s2.staged.iter().any(|e| e.path == "a.txt"));
        let id = commit(root, "init", "Test", "t@test").unwrap();
        assert!(!id.is_empty());
        fs::write(root.join("a.txt"), "hello2").unwrap();
        let d = diff(root, "working", Some("a.txt")).unwrap();
        assert!(d.contains("+hello2"));
    }

    #[test]
    fn branch_create_switch_log_show() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        init_repo_with_commit(root);
        branch_create(root, "feature").unwrap();
        branch_switch(root, "feature").unwrap();
        let branches = branch_list(root).unwrap();
        assert!(branches.iter().any(|(n, c)| n == "feature" && *c));
        fs::write(root.join("b.txt"), "b").unwrap();
        stage(root, &["b.txt".to_string()]).unwrap();
        let id = commit(root, "feat", "Test", "t@test").unwrap();
        let l = log(root, 10).unwrap();
        assert!(l.iter().any(|c| c.commit_id == id));
        let shown = show(root, &id).unwrap();
        assert!(shown.contains("b.txt") || shown.contains("+b"));
    }

    #[test]
    fn hooks_path_set_and_verified() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        init_repo(root).unwrap();
        let repo = Repository::open(root).unwrap();
        let cfg = repo.config().unwrap();
        let hooks = cfg.get_string("core.hooksPath").unwrap();
        assert!(hooks.contains(".forgebox-hooks-empty"));
        assert!(empty_hooks_dir(root).exists());
    }

    #[test]
    fn commit_validation() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        init_repo(root).unwrap();
        assert!(commit(root, "", "T", "t@t").is_err());
        fs::write(root.join("a.txt"), "x").unwrap();
        assert!(commit(root, "msg", "T", "t@t").is_err()); // nothing staged
    }

    #[test]
    fn clone_local_creates_new_repo() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        init_repo_with_commit(&src);
        clone_local(&src, &dst).unwrap();
        assert!(dst.join(".git").exists());
        assert!(src.join("a.txt").exists());
    }

    #[test]
    fn destructive_command_detection() {
        assert!(is_destructive_git_command("git push --force origin main"));
        assert!(is_destructive_git_command("git reset --hard HEAD~1"));
        assert!(is_destructive_git_command("git filter-repo --path x"));
        assert!(!is_destructive_git_command("git push origin main"));
    }
}
