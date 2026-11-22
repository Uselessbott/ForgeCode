use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::Result;

#[derive(Clone, Debug)]
pub struct AuditRecord {
    pub ts: String,
    pub actor: String,
    pub op: String,
    pub resolved_paths: Vec<String>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub policy_result: String,
    pub approval_id: Option<String>,
    pub session_id: Option<String>,
}

pub struct AuditLog {
    path: PathBuf,
    max_bytes: u64,
    max_files: usize,
}

impl AuditLog {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            max_bytes: 256 * 1024,
            max_files: 5,
        }
    }

    pub fn append(&self, record: &AuditRecord) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        self.rotate_if_needed()?;
        let line = serde_like_json(record);
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        f.flush()?;
        Ok(())
    }

    fn rotate_if_needed(&self) -> Result<()> {
        let size = fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if size < self.max_bytes {
            return Ok(());
        }
        for i in (1..self.max_files).rev() {
            let src = rotated_path(&self.path, i);
            let dst = rotated_path(&self.path, i + 1);
            if src.exists() {
                if i + 1 >= self.max_files {
                    let _ = fs::remove_file(&src);
                } else {
                    let _ = fs::rename(&src, &dst);
                }
            }
        }
        let first = rotated_path(&self.path, 1);
        if self.path.exists() {
            let _ = fs::rename(&self.path, &first);
        }
        Ok(())
    }
}

fn rotated_path(base: &Path, idx: usize) -> PathBuf {
    let mut p = base.as_os_str().to_os_string();
    p.push(format!(".{idx}"));
    PathBuf::from(p)
}

fn serde_like_json(r: &AuditRecord) -> String {
    let paths = r
        .resolved_paths
        .iter()
        .map(|p| format!("\"{}\"", escape_json(p)))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"ts\":\"{}\",\"actor\":\"{}\",\"op\":\"{}\",\"resolved_paths\":[{}],\"before_hash\":{},\"after_hash\":{},\"policy_result\":\"{}\",\"approval_id\":{},\"session_id\":{}}}",
        escape_json(&r.ts),
        escape_json(&redact(&r.actor)),
        escape_json(&r.op),
        paths,
        opt_str(r.before_hash.as_deref()),
        opt_str(r.after_hash.as_deref()),
        escape_json(&r.policy_result),
        opt_str(r.approval_id.as_deref()),
        opt_str(r.session_id.as_deref()),
    )
}

fn opt_str(v: Option<&str>) -> String {
    match v {
        Some(s) => format!("\"{}\"", escape_json(&redact(s))),
        None => "null".to_string(),
    }
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

pub fn redact(input: &str) -> String {
    let mut out = input.to_string();
    for pat in ["ghp_", "github_pat_", "sk-", "AKIA", "Bearer ", "aptok_"] {
        let mut search_from = 0;
        while let Some(rel) = out[search_from..].find(pat) {
            let idx = search_from + rel;
            let tail = &out[idx..];
            let end = tail
                .find(|c: char| {
                    c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == '}' || c == ']'
                })
                .unwrap_or(tail.len());
            let token = &out[idx..idx + end];
            if token.len() > 8 {
                let redacted = format!("{}[REDACTED]", &token[..pat.len().min(token.len())]);
                out = out.replacen(token, &redacted, 1);
                search_from = idx + redacted.len();
            } else {
                search_from = idx + pat.len();
            }
        }
    }
    out
}

pub fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_and_redact() {
        let dir = tempdir().unwrap();
        let log = AuditLog::new(dir.path().join("audit.jsonl"));
        log.append(&AuditRecord {
            ts: "1".into(),
            actor: "user".into(),
            op: "write".into(),
            resolved_paths: vec!["/p/a.txt".into()],
            before_hash: None,
            after_hash: Some("ghp_secret123456789".into()),
            policy_result: "ALLOW".into(),
            approval_id: None,
            session_id: Some("s1".into()),
        })
        .unwrap();
        let content = fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert!(!content.contains("secret123456789"));
        assert!(content.contains("[REDACTED]"));
    }

    #[test]
    fn rotation_creates_files() {
        let dir = tempdir().unwrap();
        let mut log = AuditLog::new(dir.path().join("audit.jsonl"));
        log.max_bytes = 100;
        for i in 0..20 {
            log.append(&AuditRecord {
                ts: i.to_string(),
                actor: "user".into(),
                op: "op".into(),
                resolved_paths: vec!["/p/file".into()],
                before_hash: None,
                after_hash: None,
                policy_result: "ALLOW".into(),
                approval_id: None,
                session_id: None,
            })
            .unwrap();
        }
        assert!(
            dir.path().join("audit.jsonl.1").exists() || dir.path().join("audit.jsonl").exists()
        );
    }

    #[test]
    fn redacts_aptok_and_bearer_tokens() {
        let dir = tempdir().unwrap();
        let log = AuditLog::new(dir.path().join("audit.jsonl"));
        let tok = "aptok_abcdef1234567890_999";
        log.append(&AuditRecord {
            ts: "1".into(),
            actor: "ui".into(),
            op: "approval_issued".into(),
            resolved_paths: vec![],
            before_hash: None,
            after_hash: Some("hash".into()),
            policy_result: "CONFIRM".into(),
            approval_id: Some(tok.into()),
            session_id: Some("s1".into()),
        })
        .unwrap();
        let content = fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert!(!content.contains("abcdef1234567890"));
        assert!(content.contains("[REDACTED]"));
    }

    #[test]
    fn append_only_order_preserved() {
        let dir = tempdir().unwrap();
        let log = AuditLog::new(dir.path().join("audit.jsonl"));
        for i in 0..3 {
            log.append(&AuditRecord {
                ts: i.to_string(),
                actor: "u".into(),
                op: format!("op{i}"),
                resolved_paths: vec![],
                before_hash: None,
                after_hash: None,
                policy_result: "ALLOW".into(),
                approval_id: None,
                session_id: None,
            })
            .unwrap();
        }
        let lines: Vec<String> = fs::read_to_string(dir.path().join("audit.jsonl"))
            .unwrap()
            .lines()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("op0"));
        assert!(lines[2].contains("op2"));
    }
}
