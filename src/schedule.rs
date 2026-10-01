//! When the usage service may be asked. Its rate limit is unpublished and, once hit,
//! takes minutes to clear, so every path to a request goes through these rules.

/// Shortest time between two requests that reached the same service, whatever asked
/// for the second one: the schedule, a provider switch, the panel or "refresh now".
pub const MIN_GAP_SECS: i64 = 120;
/// How often the reading is refreshed while nobody is at the machine.
pub const IDLE_POLL_SECS: i64 = 900;
/// Longer than the poll interval, the backoff and a Retry-After can ever add up to.
/// A scheduled time further away than this means the clock was set back.
pub const MAX_WAIT_SECS: i64 = 2 * 3600;

/// True once enough time has passed since the service last answered.
pub fn gap_elapsed(last_response: i64, now: i64) -> bool {
    // An answer "in the future" means the clock was set back; do not wait for it.
    last_response > now || now - last_response >= MIN_GAP_SECS
}

/// True while a wait the service imposed (rate limit, rejected token) is still running.
/// Nothing may ask during it: not the schedule, not a wake from sleep, not the user.
pub fn told_to_wait(blocked_until: i64, now: i64) -> bool {
    // A wait longer than we ever impose means the clock was set back; ignore it.
    blocked_until > now && blocked_until - now <= MAX_WAIT_SECS
}

/// Whether the regular schedule should send a request now.
///
/// `away` is true when nobody is at the machine; a reading younger than
/// `IDLE_POLL_SECS` is then good enough.
pub fn poll_due(next_poll: i64, last_response: i64, last_success: Option<i64>, away: bool, now: i64) -> bool {
    let scheduled = now >= next_poll || next_poll - now > MAX_WAIT_SECS;
    if !scheduled || !gap_elapsed(last_response, now) {
        return false;
    }
    !(away && last_success.is_some_and(|at| (0..IDLE_POLL_SECS).contains(&(now - at))))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    #[test]
    fn polls_when_the_scheduled_time_has_come() {
        assert!(!poll_due(NOW + 60, NOW - 120, Some(NOW - 120), false, NOW));
        assert!(poll_due(NOW, NOW - 180, Some(NOW - 180), false, NOW));
        assert!(poll_due(0, 0, None, false, NOW), "first run, nothing known yet");
    }

    #[test]
    fn nothing_gets_past_the_minimum_gap() {
        // Scheduled time reached (a provider switch, a forced refresh), but the service
        // answered 30 seconds ago.
        assert!(!poll_due(0, NOW - 30, Some(NOW - 30), false, NOW));
        assert!(!gap_elapsed(NOW - (MIN_GAP_SECS - 1), NOW));
        assert!(gap_elapsed(NOW - MIN_GAP_SECS, NOW));
    }

    #[test]
    fn slows_down_while_nobody_is_there() {
        let last = NOW - 300;
        assert!(poll_due(NOW - 120, last, Some(last), false, NOW), "someone is there: normal cadence");
        assert!(!poll_due(NOW - 120, last, Some(last), true, NOW), "away: five minutes old is fine");
        let old = NOW - IDLE_POLL_SECS;
        assert!(poll_due(NOW - 720, old, Some(old), true, NOW), "away: refreshed once it is 15 minutes old");
        assert!(poll_due(0, 0, None, true, NOW), "away with no reading at all still tries");
    }

    #[test]
    fn an_imposed_wait_runs_its_course() {
        assert!(told_to_wait(NOW + 300, NOW));
        assert!(told_to_wait(NOW + 1, NOW));
        assert!(!told_to_wait(NOW, NOW));
        assert!(!told_to_wait(0, NOW), "nothing imposed");
        assert!(!told_to_wait(NOW + MAX_WAIT_SECS + 1, NOW), "clock was set back");
    }

    #[test]
    fn a_clock_set_back_does_not_stall_polling() {
        assert!(poll_due(NOW + MAX_WAIT_SECS + 1, NOW - 500, Some(NOW - 500), false, NOW));
        assert!(!poll_due(NOW + MAX_WAIT_SECS, NOW - 500, Some(NOW - 500), false, NOW));
        assert!(gap_elapsed(NOW + 5000, NOW));
        assert!(poll_due(0, NOW + 5000, Some(NOW + 5000), true, NOW));
    }
}
