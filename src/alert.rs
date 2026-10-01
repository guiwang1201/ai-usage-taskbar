//! Decides when usage is worth a notification: once per threshold per window.

use crate::model::{Thresholds, Usage};
use crate::timefmt::countdown;
use crate::view::used_now;

/// How far below a threshold usage must fall before that threshold can fire again.
/// Usage only drops this much when a window resets, so a reading that wobbles around
/// a threshold does not notify twice.
const REARM_MARGIN: i64 = 5;

/// Highest level already announced for the 5-hour and the 7-day window
/// (0 none, 1 warning, 2 critical).
pub type Announced = [u8; 2];

#[derive(Clone, PartialEq, Debug)]
pub struct Alert {
    pub critical: bool,
    /// One line per window that just crossed a threshold.
    pub lines: Vec<String>,
}

/// Updates `announced` for a fresh reading and returns what should be notified, if anything.
pub fn check(announced: &mut Announced, usage: &Usage, thresholds: Thresholds, now: i64) -> Option<Alert> {
    let mut alert = Alert { critical: false, lines: Vec::new() };
    let windows = [("5 小时", usage.five_hour), ("7 天", usage.seven_day)];
    for (slot, (name, window)) in announced.iter_mut().zip(windows) {
        let used = window.map_or(0, |w| used_now(&w, now));
        *slot = (*slot).min(thresholds.level(used + REARM_MARGIN));
        let level = thresholds.level(used);
        if level > *slot {
            *slot = level;
            alert.critical |= level == 2;
            let mut line = format!("{name}额度已用 {used}%");
            if let Some(left) = window.and_then(|w| w.resets_at).map(|t| t - now).filter(|secs| *secs > 0) {
                line.push_str(&format!("，{} 后重置", countdown(left)));
            }
            alert.lines.push(line);
        }
    }
    (!alert.lines.is_empty()).then_some(alert)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Window;

    const NOW: i64 = 1_000_000;
    const T: Thresholds = Thresholds { warn: 75, crit: 90 };

    fn usage(five_hour: f64, seven_day: f64) -> Usage {
        Usage {
            five_hour: Some(Window { used_pct: five_hour, resets_at: Some(NOW + 2 * 3600 + 30) }),
            seven_day: Some(Window { used_pct: seven_day, resets_at: Some(NOW + 3 * 86_400 + 30) }),
        }
    }

    #[test]
    fn quiet_below_the_warning_threshold() {
        let mut announced = [0, 0];
        assert_eq!(check(&mut announced, &usage(74.0, 10.0), T, NOW), None);
        assert_eq!(announced, [0, 0]);
    }

    #[test]
    fn each_threshold_fires_once() {
        let mut announced = [0, 0];
        let alert = check(&mut announced, &usage(76.0, 10.0), T, NOW).unwrap();
        assert_eq!(alert, Alert { critical: false, lines: vec!["5 小时额度已用 76%，2h 后重置".to_string()] });
        assert_eq!(check(&mut announced, &usage(80.0, 10.0), T, NOW), None);

        let alert = check(&mut announced, &usage(91.0, 10.0), T, NOW).unwrap();
        assert!(alert.critical);
        assert_eq!(check(&mut announced, &usage(99.0, 10.0), T, NOW), None);
        assert_eq!(announced, [2, 0]);
    }

    #[test]
    fn jumping_straight_to_critical_is_one_alert() {
        let mut announced = [0, 0];
        let alert = check(&mut announced, &usage(10.0, 95.0), T, NOW).unwrap();
        assert_eq!(alert, Alert { critical: true, lines: vec!["7 天额度已用 95%，3d 后重置".to_string()] });
    }

    #[test]
    fn both_windows_share_one_alert() {
        let mut announced = [0, 0];
        let alert = check(&mut announced, &usage(80.0, 92.0), T, NOW).unwrap();
        assert_eq!(alert.lines.len(), 2);
        assert!(alert.critical);
    }

    #[test]
    fn a_reset_rearms_the_window() {
        let mut announced = [2, 0];
        assert_eq!(check(&mut announced, &usage(3.0, 10.0), T, NOW), None);
        assert_eq!(announced, [0, 0]);
        assert!(check(&mut announced, &usage(75.0, 10.0), T, NOW).is_some());
    }

    #[test]
    fn wobbling_around_a_threshold_does_not_repeat() {
        let mut announced = [0, 0];
        assert!(check(&mut announced, &usage(75.0, 10.0), T, NOW).is_some());
        assert_eq!(check(&mut announced, &usage(74.0, 10.0), T, NOW), None);
        assert_eq!(check(&mut announced, &usage(75.0, 10.0), T, NOW), None);
    }

    #[test]
    fn a_window_past_its_reset_counts_as_empty() {
        let mut announced = [0, 0];
        assert_eq!(check(&mut announced, &usage(95.0, 10.0), T, NOW + 3 * 3600), None);
    }
}
