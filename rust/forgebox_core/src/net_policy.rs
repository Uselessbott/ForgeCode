use crate::audit::{self, AuditLog, AuditRecord};
use crate::error::{CoreError, Result};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkMode {
    LocalOnly,
    Controlled,
    TemporaryBroader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemporaryScope {
    Project,
    Session,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorKind {
    User,
    Session,
    InstalledBinary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionResult {
    pub decision: Decision,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct AllowlistEntry {
    pub host_pattern: String,
    pub provider_kind: String,
    pub added_at: i64,
    pub added_by: String,
}

#[derive(Clone, Debug)]
pub struct NetState {
    pub mode: NetworkMode,
    pub temporary_expires_at_ms: i64,
    pub temporary_scope: Option<TemporaryScope>,
    pub temporary_scope_id: String,
    pub prior_mode: NetworkMode,
    pub allowlist: Vec<AllowlistEntry>,
}

impl Default for NetState {
    fn default() -> Self {
        Self {
            mode: NetworkMode::LocalOnly,
            temporary_expires_at_ms: 0,
            temporary_scope: None,
            temporary_scope_id: String::new(),
            prior_mode: NetworkMode::Controlled,
            allowlist: Vec::new(),
        }
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn normalize_host(host: &str) -> String {
    let h = host.trim().to_ascii_lowercase();
    // Bracketed IPv6: [::1] or [::1]:port
    if let Some(rest) = h.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return rest[..end].to_string();
        }
    }
    // Unbracketed IPv6 (contains multiple colons) - no port stripping
    if h.matches(':').count() > 1 {
        return h.trim_end_matches('.').to_string();
    }
    let h = h.split(':').next().unwrap_or(&h);
    h.trim_end_matches('.').to_string()
}

pub fn is_loopback(host: &str) -> bool {
    matches!(
        normalize_host(host).as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

fn host_matches(pattern: &str, host: &str) -> bool {
    let p = normalize_host(pattern);
    let h = normalize_host(host);
    if p.is_empty() || h.is_empty() {
        return false;
    }
    if p == h {
        return true;
    }
    if let Some(suffix) = p.strip_prefix("*.") {
        return h == suffix || h.ends_with(&format!(".{suffix}"));
    }
    false
}

pub fn effective_mode(state: &mut NetState, now: i64) -> NetworkMode {
    if matches!(state.mode, NetworkMode::TemporaryBroader)
        && state.temporary_expires_at_ms > 0
        && now >= state.temporary_expires_at_ms
    {
        state.mode = state.prior_mode;
        state.temporary_expires_at_ms = 0;
        state.temporary_scope = None;
        state.temporary_scope_id.clear();
    }
    state.mode
}

pub fn decide(
    state: &mut NetState,
    destination_host: &str,
    actor_kind: ActorKind,
    actor_id: &str,
    session_network_enabled: bool,
    now: i64,
) -> DecisionResult {
    let _ = (actor_kind, actor_id);
    if !session_network_enabled {
        return DecisionResult {
            decision: Decision::Deny,
            reason: "per-session network toggle disabled".into(),
        };
    }
    let mode = effective_mode(state, now);
    let host = normalize_host(destination_host);
    match mode {
        NetworkMode::LocalOnly => {
            if is_loopback(&host) {
                DecisionResult {
                    decision: Decision::Allow,
                    reason: "loopback allowed in LOCAL_ONLY".into(),
                }
            } else {
                // SSH hook inert in M11: no SSH config yet.
                DecisionResult {
                    decision: Decision::Deny,
                    reason: "M12 configures SSH endpoints".into(),
                }
            }
        }
        NetworkMode::Controlled => {
            if state
                .allowlist
                .iter()
                .any(|e| host_matches(&e.host_pattern, &host))
            {
                DecisionResult {
                    decision: Decision::Allow,
                    reason: "allowlist hit".into(),
                }
            } else {
                DecisionResult {
                    decision: Decision::Deny,
                    reason: "not in allowlist; switch mode or add entry".into(),
                }
            }
        }
        NetworkMode::TemporaryBroader => {
            // scope/duration enforced by effective_mode; within window allow.
            DecisionResult {
                decision: Decision::Allow,
                reason: "temporary broader mode active".into(),
            }
        }
    }
}

pub fn audit_decision(
    audit: &AuditLog,
    mode: NetworkMode,
    host: &str,
    actor_kind: ActorKind,
    actor_id: &str,
    result: &DecisionResult,
) {
    let _ = audit.append(&AuditRecord {
        ts: audit::now_iso8601(),
        actor: format!("{:?}:{actor_id}", actor_kind).to_ascii_lowercase(),
        op: "net_decision".into(),
        resolved_paths: vec![normalize_host(host)],
        before_hash: Some(format!("{mode:?}").to_ascii_lowercase()),
        after_hash: Some(format!("{:?}", result.decision).to_ascii_lowercase()),
        policy_result: result.reason.clone(),
        approval_id: None,
        session_id: if actor_id.is_empty() {
            None
        } else {
            Some(actor_id.to_string())
        },
    });
}

pub fn cap_duration_seconds(seconds: u32) -> u32 {
    let s = if seconds == 0 { 900 } else { seconds };
    s.min(3600)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn audit() -> (tempfile::TempDir, AuditLog) {
        let dir = tempdir().unwrap();
        let log = AuditLog::new(dir.path().join("audit.log"));
        (dir, log)
    }

    #[test]
    fn decision_matrix_local_only() {
        let mut st = NetState {
            mode: NetworkMode::LocalOnly,
            ..Default::default()
        };
        let d = decide(&mut st, "127.0.0.1", ActorKind::User, "u", true, 0);
        assert_eq!(d.decision, Decision::Allow);
        let d = decide(&mut st, "localhost", ActorKind::User, "u", true, 0);
        assert_eq!(d.decision, Decision::Allow);
        let d = decide(&mut st, "::1", ActorKind::User, "u", true, 0);
        assert_eq!(d.decision, Decision::Allow);
        let d = decide(&mut st, "github.com", ActorKind::Session, "s1", true, 0);
        assert_eq!(d.decision, Decision::Deny);
        assert!(d.reason.contains("M12 configures SSH endpoints"));
    }

    #[test]
    fn decision_matrix_controlled() {
        let mut st = NetState {
            mode: NetworkMode::Controlled,
            ..Default::default()
        };
        st.allowlist.push(AllowlistEntry {
            host_pattern: "github.com".into(),
            provider_kind: "github".into(),
            added_at: 1,
            added_by: "user".into(),
        });
        let d = decide(&mut st, "github.com", ActorKind::User, "u", true, 0);
        assert_eq!(d.decision, Decision::Allow);
        let d = decide(&mut st, "api.github.com", ActorKind::User, "u", true, 0);
        assert_eq!(d.decision, Decision::Deny);
        assert!(d.reason.contains("not in allowlist"));
    }

    #[test]
    fn ask_never_returned() {
        let modes = [
            NetworkMode::LocalOnly,
            NetworkMode::Controlled,
            NetworkMode::TemporaryBroader,
        ];
        for mode in modes {
            let mut st = NetState {
                mode,
                temporary_expires_at_ms: 10_000,
                prior_mode: NetworkMode::Controlled,
                ..Default::default()
            };
            for host in ["github.com", "127.0.0.1", "example.com"] {
                let d = decide(&mut st, host, ActorKind::User, "u", true, 0);
                assert_ne!(d.decision, Decision::Ask);
            }
        }
    }

    #[test]
    fn temporary_mode_expires_and_reverts() {
        let mut st = NetState {
            mode: NetworkMode::TemporaryBroader,
            temporary_expires_at_ms: 1000,
            prior_mode: NetworkMode::Controlled,
            ..Default::default()
        };
        let d = decide(&mut st, "example.com", ActorKind::User, "u", true, 500);
        assert_eq!(d.decision, Decision::Allow);
        let d = decide(&mut st, "example.com", ActorKind::User, "u", true, 1001);
        assert_eq!(d.decision, Decision::Deny);
        assert_eq!(st.mode, NetworkMode::Controlled);
    }

    #[test]
    fn per_session_toggle_overrides_mode() {
        let mut st = NetState {
            mode: NetworkMode::TemporaryBroader,
            temporary_expires_at_ms: 10_000,
            prior_mode: NetworkMode::Controlled,
            ..Default::default()
        };
        let d = decide(&mut st, "example.com", ActorKind::Session, "s1", false, 0);
        assert_eq!(d.decision, Decision::Deny);
        assert!(d.reason.contains("per-session network toggle disabled"));
    }

    #[test]
    fn audit_record_per_decision() {
        let (_dir, log) = audit();
        let mut st = NetState {
            mode: NetworkMode::Controlled,
            ..Default::default()
        };
        let d = decide(&mut st, "example.com", ActorKind::User, "u1", true, 0);
        audit_decision(
            &log,
            NetworkMode::Controlled,
            "example.com",
            ActorKind::User,
            "u1",
            &d,
        );
        let text = std::fs::read_to_string(_dir.path().join("audit.log")).unwrap();
        assert!(text.contains("\"op\":\"net_decision\""));
        assert!(text.contains("example.com"));
        assert!(text.contains("deny"));
    }

    #[test]
    fn ssh_hook_inert_local_only() {
        let mut st = NetState {
            mode: NetworkMode::LocalOnly,
            ..Default::default()
        };
        let d = decide(&mut st, "ssh.github.com", ActorKind::User, "u", true, 0);
        assert_eq!(d.decision, Decision::Deny);
        assert_eq!(d.reason, "M12 configures SSH endpoints");
    }
}
