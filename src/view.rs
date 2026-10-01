//! Turns usage data into what the widget shows. Platform-independent.

use crate::model::{FetchError, Provider, Thresholds, Usage, Window};
use crate::timefmt::countdown;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Severity {
    Ok,
    Warn,
    Crit,
}

impl Severity {
    /// From a `Thresholds::level`.
    pub fn of(level: u8) -> Severity {
        match level {
            0 => Severity::Ok,
            1 => Severity::Warn,
            _ => Severity::Crit,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Row {
    pub label: &'static str,
    /// Filled share of the bar, 0..=1. `None` draws an empty, dimmed bar.
    pub fill: Option<f32>,
    pub severity: Severity,
    pub text: String,
}

#[derive(Clone, PartialEq, Debug)]
pub struct View {
    pub provider: Provider,
    pub rows: [Row; 2],
    /// Numbers are from an earlier successful fetch; the latest one failed.
    pub stale: bool,
    /// Multi-line tooltip text.
    pub tip: String,
}

pub fn build(
    provider: Provider,
    usage: Option<&Usage>,
    error: Option<&FetchError>,
    show_remaining: bool,
    thresholds: Thresholds,
    now: i64,
) -> View {
    let mut tip = format!("IUsage · {}", provider.name());
    let rows = match usage {
        Some(usage) => {
            let rows = [
                row("5h", usage.five_hour, show_remaining, thresholds, now),
                row("7d", usage.seven_day, show_remaining, thresholds, now),
            ];
            let what = if show_remaining { "剩余" } else { "已用" };
            for (r, w) in rows.iter().zip([usage.five_hour, usage.seven_day]) {
                let Some((pct, reset)) = r.text.split_once(' ') else { continue };
                if w.is_none() {
                    continue;
                }
                tip.push_str(&format!("\n{} {what} {pct}", r.label));
                if reset != "--" {
                    tip.push_str(&format!(" · {reset} 后重置"));
                }
            }
            rows
        }
        None => {
            let status = error.map_or("…".to_string(), short_label);
            [empty_row("5h", &status), empty_row("7d", "--")]
        }
    };
    if let Some(error) = error {
        tip.push('\n');
        tip.push_str(&describe(provider, error));
    }
    View { provider, rows, stale: usage.is_some() && error.is_some(), tip }
}

/// Whole percent used right now. Once the reset time has passed the window is empty
/// again, whatever the last fetch said.
pub fn used_now(window: &Window, now: i64) -> i64 {
    match window.resets_at {
        Some(reset) if reset <= now => 0,
        _ => (window.used_pct.round() as i64).clamp(0, 100),
    }
}

fn row(label: &'static str, window: Option<Window>, show_remaining: bool, thresholds: Thresholds, now: i64) -> Row {
    let Some(window) = window else { return empty_row(label, "--") };
    let used = used_now(&window, now);
    let shown = if show_remaining { 100 - used } else { used };
    Row {
        label,
        fill: Some(shown as f32 / 100.0),
        severity: Severity::of(thresholds.level(used)),
        text: format!("{shown}% {}", countdown(window.resets_at.map_or(0, |t| t - now))),
    }
}

fn empty_row(label: &'static str, text: &str) -> Row {
    Row { label, fill: None, severity: Severity::Ok, text: text.to_string() }
}

/// Fits in the text column next to the bar.
fn short_label(error: &FetchError) -> String {
    match error {
        FetchError::NoCredentials => "未登录".to_string(),
        FetchError::Expired | FetchError::Unauthorized => "已过期".to_string(),
        FetchError::RateLimited(_) => "限流".to_string(),
        FetchError::Network(_) => "离线".to_string(),
        FetchError::Http(code) => format!("HTTP {code}"),
        FetchError::Parse => "错误".to_string(),
    }
}

/// One sentence for the tooltip and the detail panel.
pub fn describe(provider: Provider, error: &FetchError) -> String {
    let name = provider.name();
    match error {
        FetchError::NoCredentials => format!("未找到 {name} 的登录信息"),
        FetchError::Expired => format!("登录已过期，打开 {name} 即可恢复"),
        FetchError::Unauthorized => format!("登录凭据被拒绝，请在 {name} 里重新登录"),
        FetchError::RateLimited(_) => "请求过于频繁，稍后自动重试".to_string(),
        FetchError::Network(_) => "网络不通，请检查代理".to_string(),
        FetchError::Http(code) => format!("服务返回 HTTP {code}"),
        FetchError::Parse => "返回内容无法识别".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;
    const T: Thresholds = Thresholds { warn: 75, crit: 90 };

    #[test]
    fn severity_follows_the_configured_thresholds() {
        let strict = Thresholds { warn: 3, crit: 95 };
        let view = build(Provider::Claude, Some(&usage()), None, false, strict, NOW);
        assert_eq!(view.rows[0].severity, Severity::Warn);
        assert_eq!(view.rows[1].severity, Severity::Warn);
    }

    fn usage() -> Usage {
        Usage {
            five_hour: Some(Window { used_pct: 4.4, resets_at: Some(NOW + 4 * 3600 + 10) }),
            seven_day: Some(Window { used_pct: 92.0, resets_at: Some(NOW + 86_400 + 10) }),
        }
    }

    #[test]
    fn shows_used_by_default() {
        let view = build(Provider::Claude, Some(&usage()), None, false, T, NOW);
        assert_eq!(view.rows[0].text, "4% 4h");
        assert_eq!(view.rows[0].fill, Some(0.04));
        assert_eq!(view.rows[0].severity, Severity::Ok);
        assert_eq!(view.rows[1].text, "92% 1d");
        assert_eq!(view.rows[1].severity, Severity::Crit);
        assert!(!view.stale);
        assert_eq!(view.tip, "IUsage · Claude\n5h 已用 4% · 4h 后重置\n7d 已用 92% · 1d 后重置");
    }

    #[test]
    fn remaining_mode_inverts_the_number_but_not_the_severity() {
        let view = build(Provider::Claude, Some(&usage()), None, true, T, NOW);
        assert_eq!(view.rows[1].text, "8% 1d");
        assert_eq!(view.rows[1].fill, Some(0.08));
        assert_eq!(view.rows[1].severity, Severity::Crit);
    }

    #[test]
    fn window_past_its_reset_reads_as_empty() {
        let view = build(Provider::Codex, Some(&usage()), None, false, T, NOW + 5 * 3600);
        assert_eq!(view.rows[0].text, "0% --");
        assert_eq!(view.rows[0].fill, Some(0.0));
        assert_eq!(view.rows[1].text, "92% 19h");
    }

    #[test]
    fn error_without_data_shows_a_status() {
        let view = build(Provider::Claude, None, Some(&FetchError::Expired), false, T, NOW);
        assert_eq!(view.rows[0].text, "已过期");
        assert_eq!(view.rows[0].fill, None);
        assert_eq!(view.rows[1].text, "--");
        assert!(!view.stale);
        assert!(view.tip.ends_with("登录已过期，打开 Claude 即可恢复"));
    }

    #[test]
    fn errors_are_told_apart() {
        let label = |e: FetchError| build(Provider::Claude, None, Some(&e), false, T, NOW).rows[0].text.clone();
        assert_eq!(label(FetchError::RateLimited(Some(60))), "限流");
        assert_eq!(label(FetchError::Http(503)), "HTTP 503");
        assert_eq!(label(FetchError::Network("x".into())), "离线");
        assert_eq!(label(FetchError::Parse), "错误");
    }

    #[test]
    fn error_with_earlier_data_is_stale() {
        let error = FetchError::Network("timeout".into());
        let view = build(Provider::Claude, Some(&usage()), Some(&error), false, T, NOW);
        assert!(view.stale);
        assert_eq!(view.rows[0].text, "4% 4h");
    }

    #[test]
    fn loading_state() {
        let view = build(Provider::Codex, None, None, false, T, NOW);
        assert_eq!(view.rows[0].text, "…");
        assert_eq!(view.tip, "IUsage · Codex");
    }
}
