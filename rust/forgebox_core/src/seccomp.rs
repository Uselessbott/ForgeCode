#![cfg_attr(not(unix), allow(dead_code))]

// M8 seccomp spike.
// Status: compiled and unit-tested on host where supported; NOT validated on supported Android
// device/emulator in this environment (only API 28 emulator available, below minSdk 31).
// Default: OFF. This module must not claim per-process network isolation unless validated.

use crate::error::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeccompStatus {
    DisabledByDefault,
    UnsupportedPlatform,
    NotValidatedOnTarget,
    Applied,
}

#[derive(Clone, Debug)]
pub struct SeccompReport {
    pub status: SeccompStatus,
    pub detail: String,
}

pub fn default_report() -> SeccompReport {
    SeccompReport {
        status: SeccompStatus::DisabledByDefault,
        detail: "Seccomp spike is compiled-in but disabled by default; not validated on supported device".into(),
    }
}

/// Spike: attempt to apply a no-op/deny-unneeded filter shape only when explicitly enabled.
/// M8 keeps this default-off and reports honest fallback.
pub fn apply_spike(enabled: bool) -> Result<SeccompReport> {
    if !enabled {
        return Ok(default_report());
    }
    #[cfg(target_os = "linux")]
    {
        // We intentionally do not install a production filter in M8.
        // Installing seccomp in-process can break the runtime; child-process application requires
        // validated BPF and device-specific testing (R-04).
        Ok(SeccompReport {
            status: SeccompStatus::NotValidatedOnTarget,
            detail: "Seccomp enabled-request received, but M8 spike does not install filter: not validated on supported Android target; fallback to broker-only/policy controls".into(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = enabled;
        Ok(SeccompReport {
            status: SeccompStatus::UnsupportedPlatform,
            detail: "Seccomp spike unsupported on this platform".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_off_and_honest() {
        let r = default_report();
        assert_eq!(r.status, SeccompStatus::DisabledByDefault);
        assert!(r.detail.contains("not validated"));
    }

    #[test]
    fn enabled_request_falls_back_honestly() {
        let r = apply_spike(true).unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(r.status, SeccompStatus::NotValidatedOnTarget);
        #[cfg(not(target_os = "linux"))]
        assert_eq!(r.status, SeccompStatus::UnsupportedPlatform);
    }
}
