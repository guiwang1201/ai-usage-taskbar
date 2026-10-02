//! Thin C ABI over the existing shared core. No HTTP or credentials cross this API.
#![allow(dead_code)]
#[path = "../../../src/model.rs"] mod model;
#[path = "../../../src/json.rs"] mod json;
#[path = "../../../src/providers.rs"] mod providers;
#[path = "../../../src/timefmt.rs"] mod timefmt;
#[path = "../../../src/schedule.rs"] mod schedule;
#[path = "../../../src/alert.rs"] mod alert;
#[path = "../../../src/cache.rs"] mod cache;
#[path = "../../../src/settings.rs"] mod settings;
#[path = "../../../src/view.rs"] mod view;
#[path = "../../../src/panel.rs"] mod panel;

#[repr(C)]
#[derive(Default, Debug)]
pub struct MacUsage {
    pub five_hour: f64,
    pub seven_day: f64,
    pub five_reset: i64,
    pub seven_reset: i64,
    pub present: u32,
}

/// Returns 0 on success, 1 on invalid input, 2 on a malformed response.
/// On failure, output is cleared. Missing windows/reset times are represented by
/// bits 1/2 and 4/8 in `present`, not by fabricated zero-percent observations.
///
/// # Safety
/// Non-null pointers must reference readable `len` bytes and writable MacUsage.
#[no_mangle]
pub unsafe extern "C" fn iusage_parse(provider: u32, bytes: *const u8, len: usize, out: *mut MacUsage) -> i32 {
    if out.is_null() { return 1; }
    *out = MacUsage::default();
    if bytes.is_null() || len == 0 || len > 1_048_576 { return 1; }
    let provider = match provider { 0 => model::Provider::Claude, 1 => model::Provider::Codex, _ => return 1 };
    let Ok(body) = std::str::from_utf8(std::slice::from_raw_parts(bytes, len)) else { return 1; };
    let Ok(usage) = providers::parse(provider, body) else { return 2; };
    if let Some(window) = usage.five_hour {
        (*out).five_hour = window.used_pct;
        (*out).present |= 1;
        if let Some(reset) = window.resets_at { (*out).five_reset = reset; (*out).present |= 4; }
    }
    if let Some(window) = usage.seven_day {
        (*out).seven_day = window.used_pct;
        (*out).present |= 2;
        if let Some(reset) = window.resets_at { (*out).seven_reset = reset; (*out).present |= 8; }
    }
    0
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct MacGate {
    pub last_attempt: i64,
    pub next_poll: i64,
    pub blocked_until: i64,
    pub backoff: i64,
}
impl From<MacGate> for schedule::State {
    fn from(s: MacGate) -> Self { Self { last_attempt: s.last_attempt, next_poll: s.next_poll, blocked_until: s.blocked_until, backoff: s.backoff } }
}
impl From<schedule::State> for MacGate {
    fn from(s: schedule::State) -> Self { Self { last_attempt: s.last_attempt, next_poll: s.next_poll, blocked_until: s.blocked_until, backoff: s.backoff } }
}

/// # Safety
/// `gate` must point to a readable MacGate. Null gates are denied.
#[no_mangle]
pub unsafe extern "C" fn iusage_can_request(gate: *const MacGate, now: i64) -> bool {
    !gate.is_null() && schedule::State::from(*gate).can_request(now)
}

/// Reserve an attempt before I/O. Caller must persist the updated gate before
/// sending; this function itself does not write to disk or perform network I/O.
/// # Safety
/// `gate` must point to writable MacGate storage.
#[no_mangle]
pub unsafe extern "C" fn iusage_start(gate: *mut MacGate, now: i64) -> bool {
    if gate.is_null() || !iusage_can_request(gate, now) { return false; }
    let mut state = schedule::State::from(*gate);
    state.started(now);
    *gate = state.into();
    true
}

/// Complete an attempt: status 200 succeeds; 0 is a network failure.
/// Other values are HTTP status codes. Retry-After is interpreted only for 429.
/// # Safety
/// `gate` must point to writable MacGate storage.
#[no_mangle]
pub unsafe extern "C" fn iusage_complete(gate: *mut MacGate, now: i64, status: u16, retry_after: u32, poll_secs: u32) -> bool {
    if gate.is_null() { return false; }
    let mut state = schedule::State::from(*gate);
    if status == 200 {
        state.succeeded(now, poll_secs.max(30));
    } else {
        let error = match status {
            0 => model::FetchError::Network(String::new()),
            401 | 403 => model::FetchError::Unauthorized,
            429 => model::FetchError::RateLimited(Some(retry_after)),
            _ => model::FetchError::Http(status),
        };
        state.failed(&error, now);
    }
    *gate = state.into();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ffi_failure_clears_previous_reading() {
        let mut out = MacUsage { five_hour: 99., present: 1, ..Default::default() };
        unsafe { assert_eq!(iusage_parse(0, b"{}".as_ptr(), 2, &mut out), 2); }
        assert_eq!(out.present, 0);
        assert_eq!(out.five_hour, 0.);
        unsafe { assert_eq!(iusage_parse(3, b"{}".as_ptr(), 2, &mut out), 1); }
    }
    #[test]
    fn ffi_rejects_invalid_buffers_without_parsing() {
        let mut out = MacUsage::default();
        unsafe {
            assert_eq!(iusage_parse(0, std::ptr::null(), 0, &mut out), 1);
            assert_eq!(iusage_parse(0, b"{}".as_ptr(), 2, std::ptr::null_mut()), 1);
            assert_eq!(iusage_parse(0, [255u8].as_ptr(), 1, &mut out), 1);
            assert_eq!(iusage_parse(0, b"{}".as_ptr(), 1_048_577, &mut out), 1);
            assert!(!iusage_can_request(std::ptr::null(), 1000));
        }
    }
    #[test]
    fn ffi_distinguishes_absent_window_and_zero_usage() {
        let body = br#"{"five_hour":{"utilization":0},"seven_day":null}"#;
        let mut out = MacUsage::default();
        unsafe { assert_eq!(iusage_parse(0, body.as_ptr(), body.len(), &mut out), 0); }
        assert_eq!(out.present, 1);
    }
    #[test]
    fn ffi_reserves_gap_and_preserves_rejection() {
        let mut gate = MacGate::default();
        unsafe {
            assert!(iusage_start(&mut gate, 1000));
            assert!(!iusage_start(&mut gate, 1119));
            assert_eq!(gate.last_attempt, 1000);
            assert!(iusage_can_request(&gate, 1120));
        }
    }
}
