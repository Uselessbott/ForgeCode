use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::error::{CoreError, Result};

/// M8: documented bulk-overwrite threshold. Commands estimated to affect more
/// than this many files require CONFIRM. Heuristic only; exact count is
/// unknown before execution.
pub const BULK_OVERWRITE_FILE_COUNT_THRESHOLD: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Confirm,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandOrigin {
    User,
    InstalledBinary,
}

#[derive(Clone, Debug)]
pub struct PolicyAssessment {
    pub decision: PolicyDecision,
    pub reason: String,
    pub normalized_command: String,
    pub argv: Vec<String>,
    pub command_hash: String,
    pub file_count_estimate: u32,
    pub affects_outside_project: bool,
    pub uses_network: bool,
    pub creates_checkpoint: bool,
    pub classification: String,
}

#[derive(Clone, Debug)]
struct ApprovalToken {
    command_hash: String,
    session_id: String,
    expires_at: Instant,
    used: bool,
}

fn token_store() -> &'static Mutex<HashMap<String, ApprovalToken>> {
    static STORE: OnceLock<Mutex<HashMap<String, ApprovalToken>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn issue_approval_token(command_hash: &str, session_id: &str) -> Result<String> {
    let token = format!("aptok_{}_{}", command_hash, now_nanos());
    let mut store = token_store()
        .lock()
        .map_err(|_| CoreError::Internal("policy lock poisoned".into()))?;
    cleanup_expired(&mut store);
    store.insert(
        token.clone(),
        ApprovalToken {
            command_hash: command_hash.to_string(),
            session_id: session_id.to_string(),
            expires_at: Instant::now() + Duration::from_secs(60),
            used: false,
        },
    );
    Ok(token)
}

pub fn consume_approval_token(token: &str, command_hash: &str, session_id: &str) -> Result<()> {
    let mut store = token_store()
        .lock()
        .map_err(|_| CoreError::Internal("policy lock poisoned".into()))?;
    cleanup_expired(&mut store);
    let entry = store
        .get_mut(token)
        .ok_or_else(|| CoreError::InvalidArgument("approval token invalid or expired".into()))?;
    if entry.used {
        return Err(CoreError::InvalidArgument(
            "approval token already used".into(),
        ));
    }
    if entry.expires_at <= Instant::now() {
        return Err(CoreError::InvalidArgument("approval token expired".into()));
    }
    if entry.command_hash != command_hash || entry.session_id != session_id {
        return Err(CoreError::InvalidArgument(
            "approval token does not match command/session".into(),
        ));
    }
    entry.used = true;
    Ok(())
}

#[cfg(test)]
pub fn reset_token_store_for_tests() {
    if let Ok(mut store) = token_store().lock() {
        store.clear();
    }
}

fn cleanup_expired(store: &mut HashMap<String, ApprovalToken>) {
    let now = Instant::now();
    store.retain(|_, v| !v.used && v.expires_at > now);
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

pub fn evaluate_command(
    command: &str,
    actor_is_agent: bool,
    session_id: &str,
    cwd: &str,
) -> PolicyAssessment {
    evaluate_command_with_origin(
        command,
        actor_is_agent,
        session_id,
        cwd,
        CommandOrigin::User,
    )
}

pub fn evaluate_command_with_origin(
    command: &str,
    actor_is_agent: bool,
    session_id: &str,
    cwd: &str,
    origin: CommandOrigin,
) -> PolicyAssessment {
    let normalized = command.trim().to_string();
    let tokens = shell_words(&normalized);
    let argv = tokens.clone();
    let hash = simple_hash(&format!("{session_id}|{normalized}"));
    let lower = normalized.to_lowercase();
    let has_escape = has_shell_escape(&normalized);
    let unknown = tokens.is_empty();

    let mut decision = PolicyDecision::Allow;
    let mut reason = "Allowed by M8 policy".to_string();
    let mut file_count_estimate = 1u32;
    let mut affects_outside_project = false;
    let mut uses_network = looks_network(&lower);
    let mut creates_checkpoint = false;
    let mut classification = "allow".to_string();

    if matches!(origin, CommandOrigin::InstalledBinary) {
        decision = PolicyDecision::Confirm;
        classification = "installed_binary".into();
        reason =
            "This binary was installed in the Linux environment and is about to run. Continue?"
                .into();
        creates_checkpoint = true;
    } else if unknown {
        if actor_is_agent {
            decision = PolicyDecision::Deny;
            classification = "agent_unknown".into();
            reason = "Agent command could not be classified".into();
        } else {
            decision = PolicyDecision::Allow;
            classification = "empty".into();
            reason = "Empty command".into();
        }
    } else if actor_is_agent && has_escape {
        decision = PolicyDecision::Confirm;
        classification = "agent_shell_escape".into();
        reason = "Agent command contains shell escape/metacharacter".into();
        creates_checkpoint = true;
        file_count_estimate = 1;
        affects_outside_project = false;
    } else if is_recursive_delete(&tokens, &lower) {
        decision = PolicyDecision::Confirm;
        classification = "recursive_delete".into();
        reason = "Recursive delete requires confirmation".into();
        creates_checkpoint = true;
        file_count_estimate = 100;
        affects_outside_project = false;
    } else if is_project_delete(&tokens, &lower, cwd) {
        decision = PolicyDecision::Confirm;
        classification = "project_delete".into();
        reason = "Project deletion requires confirmation".into();
        creates_checkpoint = true;
        file_count_estimate = 1000;
        affects_outside_project = true;
    } else if is_bulk_overwrite(&tokens, &lower) {
        decision = PolicyDecision::Confirm;
        classification = "bulk_overwrite".into();
        reason = format!("Bulk overwrite above threshold ({BULK_OVERWRITE_FILE_COUNT_THRESHOLD} files) requires confirmation");
        creates_checkpoint = true;
        file_count_estimate = BULK_OVERWRITE_FILE_COUNT_THRESHOLD + 1;
        affects_outside_project = false;
    } else if is_git_push(&tokens, &lower) {
        decision = PolicyDecision::Confirm;
        classification = if lower.contains("--force")
            || lower.contains("--force-with-lease")
            || lower.contains(" -f")
            || lower.contains("--mirror")
        {
            "git_force_push".into()
        } else {
            "git_push".into()
        };
        reason = "Git push requires confirmation".into();
        creates_checkpoint = true;
        file_count_estimate = 1;
        affects_outside_project = false;
        uses_network = true;
    } else if is_git_reset_hard(&tokens, &lower) {
        decision = PolicyDecision::Confirm;
        classification = "git_reset_hard".into();
        reason = "git reset --hard requires confirmation".into();
        creates_checkpoint = true;
        file_count_estimate = 100;
        affects_outside_project = false;
    } else if is_history_rewrite(&tokens, &lower) {
        decision = PolicyDecision::Confirm;
        classification = "history_rewrite".into();
        reason = "History rewrite requires confirmation".into();
        creates_checkpoint = true;
        file_count_estimate = 100;
        affects_outside_project = false;
    } else if is_hooks_change(&tokens, &lower) {
        decision = PolicyDecision::Confirm;
        classification = "hooks_change".into();
        reason = "Hooks change requires confirmation".into();
        creates_checkpoint = true;
        file_count_estimate = 1;
        affects_outside_project = false;
    } else if is_auth_setting_change(&tokens, &lower) {
        decision = PolicyDecision::Confirm;
        classification = "auth_setting_change".into();
        reason = "Auth setting change requires confirmation".into();
        creates_checkpoint = true;
        file_count_estimate = 1;
        affects_outside_project = false;
    } else if writes_sensitive_path(&tokens, &lower, cwd) {
        decision = PolicyDecision::Confirm;
        classification = "sensitive_path_write".into();
        reason = "Write to Config/internal/outside project requires confirmation".into();
        creates_checkpoint = true;
        file_count_estimate = 1;
        affects_outside_project = true;
    } else if is_dangerous_tool(&tokens, &lower) {
        decision = PolicyDecision::Confirm;
        classification = "dangerous_tool".into();
        reason = "Dangerous system/file command requires confirmation".into();
        creates_checkpoint = true;
        file_count_estimate = 100;
        affects_outside_project = true;
    } else if actor_is_agent && is_interpreter_inline_exec(&tokens) {
        decision = PolicyDecision::Confirm;
        classification = "agent_interpreter_exec".into();
        reason = "Agent inline interpreter execution requires confirmation".into();
        creates_checkpoint = true;
    } else if actor_is_agent && is_unknown_construct(&tokens, &lower) {
        decision = PolicyDecision::Deny;
        classification = "agent_unknown_construct".into();
        reason = "Agent command could not be classified with high confidence".into();
    } else if has_escape && !actor_is_agent {
        // User-initiated shell escapes in interactive terminal are allowed by M8
        // (they are normal shell usage); agent escapes are CONFIRM above.
        classification = "user_shell_escape".into();
    }

    PolicyAssessment {
        decision,
        reason,
        normalized_command: normalized,
        argv,
        command_hash: hash,
        file_count_estimate,
        affects_outside_project,
        uses_network,
        creates_checkpoint,
        classification,
    }
}

pub fn shell_words(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    while let Some(c) = chars.next() {
        if escape {
            cur.push(c);
            escape = false;
            continue;
        }
        if c == '\\' && !in_single {
            escape = true;
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if c.is_whitespace() && !in_single && !in_double {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn has_shell_escape(cmd: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut esc = false;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if esc {
            esc = false;
            i += 1;
            continue;
        }
        if c == '\\' && !in_single {
            esc = true;
            i += 1;
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            i += 1;
            continue;
        }
        if !in_single && !in_double {
            if matches!(c, ';' | '`' | '|') {
                return true;
            }
            if c == '&' && chars.get(i + 1) == Some(&'&') {
                return true;
            }
            if c == '|' && chars.get(i + 1) == Some(&'|') {
                return true;
            }
            if c == '$' && chars.get(i + 1) == Some(&'(') {
                return true;
            }
            if c == '<' && chars.get(i + 1) == Some(&'(') {
                return true;
            }
            if c == '>' && chars.get(i + 1) == Some(&'(') {
                return true;
            }
        }
        if !in_single && c == '$' && chars.get(i + 1) == Some(&'(') {
            return true;
        }
        i += 1;
    }
    false
}

fn is_recursive_delete(tokens: &[String], lower: &str) -> bool {
    let first = tokens.first().map(|s| s.as_str()).unwrap_or("");
    if first == "rm" {
        return tokens
            .iter()
            .any(|t| t.starts_with('-') && (t.contains('r') || t.contains('R')));
    }
    if first == "rmdir" && tokens.iter().any(|t| t == "-p") {
        return true;
    }
    if first == "find" && (lower.contains(" -delete") || lower.contains("-exec rm")) {
        return true;
    }
    lower.contains("rm -rf") || lower.contains("rm -fr")
}
fn is_project_delete(tokens: &[String], lower: &str, cwd: &str) -> bool {
    if is_recursive_delete(tokens, lower) {
        return lower.contains("/projects/")
            || lower.contains("projects/")
            || cwd.contains("/ForgeBox/Projects")
            || lower.contains(".."); // conservative: deleting upward from project context
    }
    false
}
fn is_bulk_overwrite(_tokens: &[String], lower: &str) -> bool {
    // M8 threshold: heuristic; commands likely affecting >BULK_OVERWRITE_FILE_COUNT_THRESHOLD files.
    lower.contains("sed -i")
        || lower.contains("perl -pi")
        || lower.contains("xargs rm")
        || lower.contains("find . -exec")
        || lower.contains("git clean -fd")
        || (lower.contains("> ") && lower.contains('*'))
        || (lower.contains("tee ") && lower.contains('*'))
        || lower.contains("rsync -a --delete")
}
fn is_git_push(tokens: &[String], _lower: &str) -> bool {
    tokens.first().map(|s| s == "git").unwrap_or(false) && tokens.iter().any(|t| t == "push")
}

fn is_git_reset_hard(tokens: &[String], _lower: &str) -> bool {
    tokens.len() >= 3
        && tokens[0] == "git"
        && tokens[1] == "reset"
        && tokens.iter().any(|t| t == "--hard")
}
fn is_history_rewrite(tokens: &[String], lower: &str) -> bool {
    lower.contains("filter-branch")
        || lower.contains("filter-repo")
        || (tokens.first().map(|s| s.as_str()) == Some("git")
            && tokens.iter().any(|t| t == "rebase")
            && tokens.iter().any(|t| t == "-i" || t == "--interactive"))
}
fn is_hooks_change(tokens: &[String], lower: &str) -> bool {
    lower.contains("core.hookspath")
        || lower.contains(".git/hooks")
        || (tokens.first().map(|s| s.as_str()) == Some("git")
            && tokens.iter().any(|t| t == "config")
            && lower.contains("hook"))
        || (matches!(tokens.first().map(|s| s.as_str()), Some("cp" | "mv" | "ln"))
            && lower.contains(".git/hooks"))
}
fn is_auth_setting_change(tokens: &[String], lower: &str) -> bool {
    if tokens.first().map(|s| s.as_str()) == Some("git") && tokens.iter().any(|t| t == "config") {
        return lower.contains("credential")
            || lower.contains("user.email")
            || lower.contains("user.name")
            || lower.contains("http.") && lower.contains("proxy");
    }
    lower.contains("credential.helper") || (lower.contains("auth") && lower.contains("config"))
}
fn writes_sensitive_path(tokens: &[String], lower: &str, cwd: &str) -> bool {
    let sensitive = lower.contains("/config/")
        || lower.contains("/internal/")
        || lower.contains(" config/")
        || lower.contains(" internal/")
        || lower.contains("../")
        || lower.contains("/../");
    if sensitive {
        return true;
    }
    // writes outside assigned project: absolute path not under cwd/Projects scope
    if matches!(
        tokens.first().map(|s| s.as_str()),
        Some("cp" | "mv" | "tee" | "touch" | "mkdir")
    ) {
        for t in tokens.iter().skip(1) {
            if t.starts_with('/') && !t.contains("/ForgeBox/Projects/") && !t.starts_with(cwd) {
                return true;
            }
        }
    }
    let _ = cwd;
    false
}
fn is_dangerous_tool(tokens: &[String], _lower: &str) -> bool {
    let first = tokens.first().map(|s| s.as_str()).unwrap_or("");
    matches!(
        first,
        "dd" | "shred" | "mount" | "umount" | "mkfs" | "mkfs.ext4" | "mkfs.vfat"
    ) || first.starts_with("mkfs")
        || (first == "chmod" && tokens.iter().any(|t| t == "-R"))
        || (first == "chown" && tokens.iter().any(|t| t == "-R"))
}

fn is_interpreter_inline_exec(tokens: &[String]) -> bool {
    let first = tokens.first().map(|s| s.as_str()).unwrap_or("");
    matches!(
        first,
        "python" | "python3" | "sh" | "bash" | "node" | "perl" | "ruby"
    ) && tokens
        .iter()
        .any(|t| matches!(t.as_str(), "-c" | "-e" | "-e" | "--eval"))
}
fn is_pipeline_to_interpreter(lower: &str) -> bool {
    let interpreters = [
        "| sh",
        "| bash",
        "| python",
        "| python3",
        "| perl -e",
        "| perl",
        "| node -e",
        "| node",
        "| ruby -e",
        "| ruby",
        "| zsh",
    ];
    interpreters.iter().any(|p| lower.contains(p))
}

fn looks_network(lower: &str) -> bool {
    [
        "curl ",
        "wget ",
        "ssh ",
        "scp ",
        "git push",
        "git fetch",
        "git clone",
        "npm install",
        "pip install",
        "cargo fetch",
    ]
    .iter()
    .any(|p| lower.contains(p))
}
fn is_unknown_construct(tokens: &[String], lower: &str) -> bool {
    let first = tokens.first().map(|s| s.as_str()).unwrap_or("");
    let known = [
        "ls", "pwd", "cat", "echo", "cd", "git", "python", "python3", "sh", "bash", "grep", "find",
        "sed", "awk", "head", "tail", "mkdir", "touch", "cp", "mv", "rm", "chmod", "chown", "true",
        "false", "test", "[", "printf", "env", "export", "which", "id", "uname", "date", "wc",
        "sort", "uniq", "tee", "xargs", "rmdir",
    ];
    if !known.contains(&first) {
        return true;
    }
    lower.contains("$(") || lower.contains('`') || lower.contains("<(") || lower.contains(">(")
}
fn simple_hash(s: &str) -> String {
    let mut h: u64 = 14695981039346656037;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_recursive_delete_confirm() {
        for cmd in [
            "rm -rf build",
            "rm -r dir",
            "rmdir -p a/b",
            "find . -delete",
            "find . -exec rm {} \\;",
        ] {
            let a = evaluate_command(cmd, false, "s1", "/ForgeBox/Projects/p");
            assert_eq!(a.decision, PolicyDecision::Confirm, "{cmd}");
        }
    }
    #[test]
    fn classify_force_push_variants() {
        for cmd in [
            "git push --force origin main",
            "git push --force-with-lease",
            "git push -f origin main",
            "git push --mirror origin",
            "git push origin main",
        ] {
            let a = evaluate_command(cmd, false, "s1", "/p");
            assert_eq!(a.decision, PolicyDecision::Confirm, "{cmd}");
        }
    }
    #[test]
    fn classify_git_reset_hard_confirm() {
        let a = evaluate_command("git reset --hard HEAD~1", false, "s1", "/p");
        assert_eq!(a.decision, PolicyDecision::Confirm);
    }
    #[test]
    fn classify_history_rewrite_confirm() {
        for cmd in [
            "git filter-branch --all",
            "git filter-repo --path x",
            "git rebase -i HEAD~3",
        ] {
            let a = evaluate_command(cmd, false, "s1", "/p");
            assert_eq!(a.decision, PolicyDecision::Confirm, "{cmd}");
        }
    }
    #[test]
    fn classify_hooks_and_auth() {
        for cmd in [
            "git config core.hooksPath /tmp/h",
            "cp hook .git/hooks/pre-commit",
            "git config credential.helper store",
            "git config user.email a@b.com",
        ] {
            let a = evaluate_command(cmd, false, "s1", "/p");
            assert_eq!(a.decision, PolicyDecision::Confirm, "{cmd}");
        }
    }
    #[test]
    fn classify_sensitive_path_and_dangerous() {
        for cmd in [
            "cp x /data/Config/x",
            "dd if=/dev/zero of=x",
            "shred -u x",
            "chmod -R 777 .",
            "chown -R u .",
            "mount /dev/x /mnt",
        ] {
            let a = evaluate_command(cmd, false, "s1", "/ForgeBox/Projects/p");
            assert_eq!(a.decision, PolicyDecision::Confirm, "{cmd}");
        }
    }
    #[test]
    fn classify_bulk_overwrite_confirm() {
        let a = evaluate_command("sed -i 's/a/b/g' *.txt", false, "s1", "/p");
        assert_eq!(a.decision, PolicyDecision::Confirm);
        assert!(a.file_count_estimate > BULK_OVERWRITE_FILE_COUNT_THRESHOLD);
    }
    #[test]
    fn agent_unknown_deny() {
        let a = evaluate_command("weirdtool --doit", true, "s1", "/p");
        assert_eq!(a.decision, PolicyDecision::Deny);
    }
    #[test]
    fn agent_escape_confirm() {
        let a = evaluate_command("echo hi; rm -rf /", true, "s1", "/p");
        assert_eq!(a.decision, PolicyDecision::Confirm);
    }
    #[test]
    fn parser_quotes_escapes_subshell_pipeline() {
        let words = shell_words("echo \"hello world\" 'semi;colon' plain");
        assert_eq!(words, vec!["echo", "hello world", "semi;colon", "plain"]);
        let quoted_meta = shell_words("echo \"a; b | c && d\"");
        assert_eq!(quoted_meta, vec!["echo", "a; b | c && d"]);
        assert!(has_shell_escape("echo $(cat /etc/passwd)"));
        assert!(has_shell_escape("echo `id`"));
        assert!(has_shell_escape("cat a | sh"));
        assert!(has_shell_escape("cmd <(cat x)"));
        assert!(has_shell_escape("cmd >(cat x)"));
        assert!(!has_shell_escape("echo ';' \"|\""));
        let nested = shell_words("echo $(echo $(pwd))");
        assert_eq!(nested[0], "echo");
    }
    #[test]
    fn parser_escaped_metacharacters_and_whitespace() {
        let words = shell_words("echo a\\;b c\\|d e\\&\\&f");
        assert_eq!(words, vec!["echo", "a;b", "c|d", "e&&f"]);
        let piped = shell_words("  cat a  |  grep x  ");
        assert!(piped.contains(&"|".to_string()));
    }
    #[test]
    fn unknown_construct_default_confirm_for_agent_allow_for_user() {
        let agent = evaluate_command("python -c 'print(1)'", true, "s1", "/p");
        assert!(matches!(
            agent.decision,
            PolicyDecision::Deny | PolicyDecision::Confirm
        ));
        let user = evaluate_command("python -c 'print(1)'", false, "s1", "/p");
        assert_ne!(user.decision, PolicyDecision::Deny);
        let agent_unknown = evaluate_command("obscurecmd --flag", true, "s1", "/p");
        assert_eq!(agent_unknown.decision, PolicyDecision::Deny);
        let user_unknown = evaluate_command("obscurecmd --flag", false, "s1", "/p");
        assert_ne!(user_unknown.decision, PolicyDecision::Deny);
    }
    #[test]
    fn agent_cannot_self_approve_by_embedding_token() {
        let a = evaluate_command("rm -rf / --token aptok_deadbeef_123", true, "s1", "/p");
        assert_eq!(a.decision, PolicyDecision::Confirm);
        assert!(consume_approval_token("aptok_deadbeef_123", &a.command_hash, "s1").is_err());
        let b = evaluate_command(
            "git push --force origin main aptok_aaaaaaaa_1",
            true,
            "s1",
            "/p",
        );
        assert_eq!(b.decision, PolicyDecision::Confirm);
        assert!(consume_approval_token("aptok_aaaaaaaa_1", &b.command_hash, "s1").is_err());
    }

    #[test]
    fn installed_binary_origin_requires_confirm() {
        let a = evaluate_command_with_origin(
            "codex --version",
            false,
            "s1",
            "/p",
            CommandOrigin::InstalledBinary,
        );
        assert_eq!(a.decision, PolicyDecision::Confirm);
        assert_eq!(a.classification, "installed_binary");
        assert!(a.reason.contains("installed in the Linux environment"));
    }

    #[test]
    fn user_package_install_allow_unless_other_rule() {
        let a = evaluate_command(
            "npm install -g @mmmbuto/codex-cli-termux",
            false,
            "s1",
            "/p",
        );
        assert_eq!(a.decision, PolicyDecision::Allow);
    }

    #[test]
    fn full_confirm_set_samples() {
        let cases = [
            "rm -rf /tmp/x",
            "find . -name '*.tmp' -delete",
            "git push --mirror origin",
            "git filter-branch --all",
            "git config core.hooksPath /tmp/hooks",
            "cp hook .git/hooks/pre-commit",
            "git config credential.helper store",
            "dd if=/dev/zero of=disk.img",
            "chmod -R 777 .",
            "chown -R u:g .",
            "mount /dev/sda1 /mnt",
            "mkfs.ext4 /dev/sda1",
            "shred -u secrets.txt",
        ];
        for cmd in cases {
            let a = evaluate_command(cmd, false, "s1", "/ForgeBox/Projects/p");
            assert_eq!(a.decision, PolicyDecision::Confirm, "{cmd}");
        }
    }
    #[test]
    fn token_single_use_and_binding() {
        reset_token_store_for_tests();
        let t = issue_approval_token("abc", "s1").unwrap();
        consume_approval_token(&t, "abc", "s1").unwrap();
        assert!(consume_approval_token(&t, "abc", "s1").is_err());
        let t2 = issue_approval_token("def", "s1").unwrap();
        assert!(consume_approval_token(&t2, "def", "s2").is_err());
    }
}
