//! Last good reading per provider, kept on disk. A restart shows it straight away and
//! does not need to ask the service again while it is still fresh.

use std::path::Path;

use tinyjson::JsonValue;

use crate::alert::Announced;
use crate::json::{self, get, num};
use crate::model::{Provider, Usage, Window};
use crate::schedule::State;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Source { #[default] Unknown, OAuth, Web, Codex }

impl Source {
    pub fn key(self) -> &'static str {
        match self { Self::Unknown => "unknown", Self::OAuth => "oauth", Self::Web => "web", Self::Codex => "codex" }
    }
    pub fn label(self) -> &'static str {
        match self { Self::Unknown => "旧缓存", Self::OAuth => "Claude Code", Self::Web => "Claude 网页", Self::Codex => "Codex" }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Entry {
    pub source: Source,
    pub usage: Usage,
    /// Unix seconds of the fetch that produced `usage`.
    pub fetched_at: i64,
    /// Which thresholds have already been notified, so a restart does not repeat them.
    pub announced: Announced,
}

/// Indexed by `Provider::index`.
pub type Cache = [Option<Entry>; 2];

/// Returns true only on the first confirmed authentication failure in an episode.
pub fn mark_web_expired(expired: &mut bool, error: &crate::model::FetchError) -> bool {
    if matches!(error, crate::model::FetchError::WebUnauthorized) && !*expired {
        *expired = true;
        true
    } else { false }
}

/// Everything kept across restarts.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Stored {
    pub web_expired: bool,
    pub cache: Cache,
    pub schedule: [State; 2],
}

/// A missing or malformed file is an empty cache.
pub fn load(path: &Path) -> Stored {
    let mut stored = Stored::default();
    let Ok(text) = json::read_small_file(path) else { return stored };
    let Some(root) = json::parse(&text) else { return stored };
    stored.web_expired = matches!(get(&root, "web_expired"), Some(JsonValue::Boolean(true)));
    for provider in Provider::ALL {
        stored.cache[provider.index()] = get(&root, provider.key()).and_then(entry);
        stored.schedule[provider.index()].last_attempt = stored.cache[provider.index()].map_or(0, |e| e.fetched_at);
    }
    if let Some(JsonValue::Array(times)) = get(&root, "blocked_until") {
        for (slot, time) in stored.schedule.iter_mut().zip(times) {
            slot.blocked_until = num(time).map_or(0, |n| n as i64);
        }
    }
    if let Some(JsonValue::Array(states)) = get(&root, "schedule") {
        for (slot, state) in stored.schedule.iter_mut().zip(states) {
            let read = |key| get(state, key).and_then(num).map(|n| n as i64);
            slot.last_attempt = read("last_attempt").unwrap_or(slot.last_attempt);
            slot.next_poll = read("next_poll").unwrap_or(0);
            slot.blocked_until = read("blocked_until").unwrap_or(slot.blocked_until);
            slot.backoff = read("backoff").unwrap_or(0).clamp(0, 960);
        }
    }
    stored
}

pub fn save(path: &Path, stored: &Stored) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let cache = &stored.cache;
    let mut entries: Vec<String> = Provider::ALL
        .into_iter()
        .map(|provider| {
            let value = match &cache[provider.index()] {
                Some(e) => format!(
                    "{{\"source\":\"{}\",\"fetched_at\":{},\"five_hour\":{},\"seven_day\":{},\"announced\":[{},{}]}}",
                    e.source.key(),
                    e.fetched_at,
                    window_json(e.usage.five_hour),
                    window_json(e.usage.seven_day),
                    e.announced[0],
                    e.announced[1]
                ),
                None => "null".to_string(),
            };
            format!("\"{}\":{value}", provider.key())
        })
        .collect();
    // Keep the old cooldown field readable by an older executable as well.
    entries.push(format!("\"web_expired\":{}", stored.web_expired));
    entries.push(format!("\"blocked_until\":[{},{}]", stored.schedule[0].blocked_until, stored.schedule[1].blocked_until));
    let states = stored.schedule.map(|s| format!(
        "{{\"last_attempt\":{},\"next_poll\":{},\"blocked_until\":{},\"backoff\":{}}}",
        s.last_attempt, s.next_poll, s.blocked_until, s.backoff
    ));
    entries.push(format!("\"schedule\":[{}]", states.join(",")));
    json::write_file(path, &format!("{{{}}}\n", entries.join(",")))
}

fn window_json(window: Option<Window>) -> String {
    match window {
        Some(w) => {
            let resets_at = w.resets_at.map_or("null".to_string(), |t| t.to_string());
            format!("{{\"used_pct\":{},\"resets_at\":{resets_at}}}", w.used_pct)
        }
        None => "null".to_string(),
    }
}

fn entry(value: &JsonValue) -> Option<Entry> {
    let mut announced: Announced = [0, 0];
    if let Some(JsonValue::Array(levels)) = get(value, "announced") {
        for (slot, level) in announced.iter_mut().zip(levels) {
            *slot = num(level).map_or(0, |n| n.clamp(0.0, 3.0) as u8);
        }
    }
    Some(Entry {
        source: match get(value, "source").and_then(json::text) {
            Some("oauth") => Source::OAuth, Some("web") => Source::Web, Some("codex") => Source::Codex, _ => Source::Unknown,
        },
        announced,
        fetched_at: timestamp(get(value, "fetched_at")?)?,
        usage: Usage {
            five_hour: get(value, "five_hour").and_then(window),
            seven_day: get(value, "seven_day").and_then(window),
        },
    })
}

fn window(value: &JsonValue) -> Option<Window> {
    Some(Window {
        used_pct: num(get(value, "used_pct")?).filter(|n| (0.0..=100.0).contains(n))?,
        resets_at: get(value, "resets_at").and_then(timestamp),
    })
}

fn timestamp(value: &JsonValue) -> Option<i64> {
    // Keep corrupt cache values out of date arithmetic (through year 9999).
    num(value).filter(|n| (0.0..=253_402_300_799.0).contains(n)).map(|n| n as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_expiry_is_distinct_from_access_blocks_and_rate_limits() {
        use crate::model::FetchError;
        let mut expired = false;
        for error in [FetchError::WebForbidden, FetchError::RateLimited(Some(600)), FetchError::Network("timeout".into()), FetchError::WebOrganization] {
            assert!(!mark_web_expired(&mut expired, &error));
            assert!(!expired);
        }
        assert!(mark_web_expired(&mut expired, &FetchError::WebUnauthorized));
        assert!(!mark_web_expired(&mut expired, &FetchError::WebUnauthorized));
        let path = std::env::temp_dir().join(format!("iusage-expired-{}.json", std::process::id()));
        let mut stored = Stored { web_expired: expired, ..Stored::default() };
        stored.schedule[0].failed(&FetchError::WebUnauthorized, 1000);
        save(&path, &stored).unwrap();
        let mut restored = load(&path);
        assert!(!mark_web_expired(&mut restored.web_expired, &FetchError::WebUnauthorized));
        restored.web_expired = false; // A user update allows a new episode, not a cooldown bypass.
        assert!(!restored.schedule[0].can_request(1100));
        assert!(mark_web_expired(&mut restored.web_expired, &FetchError::WebUnauthorized));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn round_trips() {
        let path = std::env::temp_dir().join(format!("iusage-cache-test-{}.json", std::process::id()));
        let stored = Stored {
            web_expired: true,
            cache: [
                Some(Entry {
                    source: Source::Web,
                    fetched_at: 1_790_000_000,
                    announced: [1, 2],
                    usage: Usage {
                        five_hour: Some(Window { used_pct: 17.5, resets_at: Some(1_790_875_800) }),
                        seven_day: Some(Window { used_pct: 64.0, resets_at: None }),
                    },
                }),
                None,
            ],
            schedule: [State { last_attempt: 1_790_000_000, blocked_until: 1_790_000_300, backoff: 300, next_poll: 1_790_000_300 }, State::default()],
        };
        save(&path, &stored).unwrap();
        assert_eq!(load(&path), stored);

        let other = Stored {
            web_expired: false,
            cache: [None, Some(Entry { source: Source::Unknown, fetched_at: 5, announced: [0, 0], usage: Usage::default() })],
            schedule: [State::default(), State { blocked_until: 77, ..State::default() }],
        };
        save(&path, &other).unwrap();
        assert_eq!(load(&path), other);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn long_server_cooldown_survives_restart_without_a_reading() {
        let path = std::env::temp_dir().join(format!("iusage-long-wait-{}.json", std::process::id()));
        let mut stored = Stored::default();
        stored.schedule[0].failed(&crate::model::FetchError::RateLimited(Some(86_400)), 1_790_000_000);
        save(&path, &stored).unwrap();
        let restored = load(&path);
        assert!(!restored.schedule[0].can_request(1_790_080_000));
        assert_eq!(restored, stored);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn unreadable_file_is_an_empty_cache() {
        let path = std::env::temp_dir().join(format!("iusage-cache-bad-{}.json", std::process::id()));
        assert_eq!(load(&path), Stored::default());
        std::fs::write(&path, "{\"claude\":{\"five_hour\":{}},\"codex\":7").unwrap();
        assert_eq!(load(&path), Stored::default());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_file_written_before_blocked_until_existed_still_loads() {
        let path = std::env::temp_dir().join(format!("iusage-cache-old-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"claude":{"fetched_at":9,"five_hour":null,"seven_day":null},"codex":null}"#).unwrap();
        let stored = load(&path);
        assert_eq!(stored.cache[0].map(|e| e.fetched_at), Some(9));
        assert_eq!(stored.schedule[0].last_attempt, 9);
        assert_eq!(stored.schedule[0].blocked_until, 0);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn restores_legacy_cooldowns_without_usage_data() {
        let path = std::env::temp_dir().join(format!("iusage-cache-wait-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"blocked_until":[1000300,0]}"#).unwrap();
        let stored = load(&path);
        assert!(!stored.schedule[0].can_request(1_000_000));
        assert!(stored.schedule[0].can_request(1_000_300));
        std::fs::remove_file(path).unwrap();
    }
}
