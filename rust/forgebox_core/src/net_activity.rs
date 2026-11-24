use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::audit::{self, AuditLog, AuditRecord};
use crate::net_policy::ActorKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityStatus {
    Pending,
    InFlight,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct ActivityEvent {
    pub ts: i64,
    pub destination_host: String,
    pub actor_kind: ActorKind,
    pub actor_id: String,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub status: ActivityStatus,
    pub cancel_token: String,
    pub reason: String,
}

pub struct ActivityLog {
    ring: VecDeque<ActivityEvent>,
    cap: usize,
    in_flight: HashMap<String, usize>,
    next_id: AtomicU64,
}

impl ActivityLog {
    pub fn new(cap: usize) -> Self {
        Self {
            ring: VecDeque::new(),
            cap: cap.max(1),
            in_flight: HashMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn next_token(&self) -> String {
        format!("net-{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    pub fn record(&mut self, mut event: ActivityEvent) {
        if event.cancel_token.is_empty() {
            event.cancel_token = self.next_token();
        }
        if matches!(
            event.status,
            ActivityStatus::Pending | ActivityStatus::InFlight
        ) {
            self.in_flight
                .insert(event.cancel_token.clone(), self.ring.len());
        }
        self.ring.push_back(event);
        while self.ring.len() > self.cap {
            let ev = self.ring.pop_front();
            if let Some(ev) = ev {
                self.in_flight.remove(&ev.cancel_token);
            }
            // rebuild indices
            self.in_flight.clear();
            for (i, ev) in self.ring.iter().enumerate() {
                if matches!(
                    ev.status,
                    ActivityStatus::Pending | ActivityStatus::InFlight
                ) {
                    self.in_flight.insert(ev.cancel_token.clone(), i);
                }
            }
        }
    }

    pub fn cancel(&mut self, token: &str, audit: &AuditLog) -> Option<ActivityStatus> {
        let idx = *self.in_flight.get(token)?;
        let ev = self.ring.get_mut(idx)?;
        ev.status = ActivityStatus::Cancelled;
        ev.reason = "cancelled by user".into();
        let snapshot = ev.clone();
        self.in_flight.remove(token);
        let _ = audit.append(&AuditRecord {
            ts: audit::now_iso8601(),
            actor: format!("{:?}:{}", snapshot.actor_kind, snapshot.actor_id).to_ascii_lowercase(),
            op: "net_cancel".into(),
            resolved_paths: vec![snapshot.destination_host],
            before_hash: Some(token.to_string()),
            after_hash: Some("cancelled".into()),
            policy_result: snapshot.reason,
            approval_id: None,
            session_id: if snapshot.actor_id.is_empty() {
                None
            } else {
                Some(snapshot.actor_id)
            },
        });
        Some(ActivityStatus::Cancelled)
    }

    pub fn list(&self, limit: usize) -> Vec<ActivityEvent> {
        let n = limit.min(self.ring.len());
        self.ring.iter().rev().take(n).cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ev(status: ActivityStatus, token: &str) -> ActivityEvent {
        ActivityEvent {
            ts: 1,
            destination_host: "example.com".into(),
            actor_kind: ActorKind::User,
            actor_id: "u".into(),
            bytes_up: 0,
            bytes_down: 0,
            status,
            cancel_token: token.into(),
            reason: String::new(),
        }
    }

    #[test]
    fn records_and_cancel() {
        let dir = tempdir().unwrap();
        let audit = AuditLog::new(dir.path().join("audit.log"));
        let mut log = ActivityLog::new(10);
        log.record(ev(ActivityStatus::InFlight, "t1"));
        log.record(ev(ActivityStatus::Succeeded, "t2"));
        let st = log.cancel("t1", &audit).unwrap();
        assert_eq!(st, ActivityStatus::Cancelled);
        let listed = log.list(10);
        assert!(listed
            .iter()
            .any(|e| e.cancel_token == "t1" && matches!(e.status, ActivityStatus::Cancelled)));
        let text = std::fs::read_to_string(dir.path().join("audit.log")).unwrap();
        assert!(text.contains("\"op\":\"net_cancel\""));
    }

    #[test]
    fn ring_bounded() {
        let mut log = ActivityLog::new(2);
        log.record(ev(ActivityStatus::Succeeded, "a"));
        log.record(ev(ActivityStatus::Succeeded, "b"));
        log.record(ev(ActivityStatus::Succeeded, "c"));
        assert_eq!(log.len(), 2);
        let listed = log.list(10);
        assert!(listed.iter().any(|e| e.cancel_token == "c"));
        assert!(!listed.iter().any(|e| e.cancel_token == "a"));
    }
}
