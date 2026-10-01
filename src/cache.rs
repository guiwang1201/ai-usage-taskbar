//! Last good reading per provider, kept on disk. A restart shows it straight away and
//! does not need to ask the service again while it is still fresh.

use std::path::Path;

use tinyjson::JsonValue;

use crate::alert::Announced;
use crate::json::{self, get, num};
use crate::model::{Provider, Usage, Window};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Entry {
    pub usage: Usage,
    /// Unix seconds of the fetch that produced `usage`.
    pub fetched_at: i64,
    /// Which thresholds have already been notified, so a restart does not repeat them.
    pub announced: Announced,
}

/// Indexed by `Provider::index`.
pub type Cache = [Option<Entry>; 2];

/// Everything kept across restarts.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Stored {
    pub cache: Cache,
    /// Unix seconds before which the service must not be asked again, per provider:
    /// set when it rate limited or rejected us, so that a restart does not ask early.
    pub blocked_until: [i64; 2],
}

/// A missing or malformed file is an empty cache.
pub fn load(path: &Path) -> Stored {
    let mut stored = Stored::default();
    let Ok(text) = std::fs::read_to_string(path) else { return stored };
    let Some(root) = json::parse(&text) else { return stored };
    for provider in Provider::ALL {
        stored.cache[provider.index()] = get(&root, provider.key()).and_then(entry);
    }
    if let Some(JsonValue::Array(times)) = get(&root, "blocked_until") {
        for (slot, time) in stored.blocked_until.iter_mut().zip(times) {
            *slot = num(time).map_or(0, |n| n as i64);
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
                    "{{\"fetched_at\":{},\"five_hour\":{},\"seven_day\":{},\"announced\":[{},{}]}}",
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
    entries.push(format!("\"blocked_until\":[{},{}]", stored.blocked_until[0], stored.blocked_until[1]));
    std::fs::write(path, format!("{{{}}}\n", entries.join(",")))
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
            *slot = num(level).map_or(0, |n| n.clamp(0.0, 2.0) as u8);
        }
    }
    Some(Entry {
        announced,
        fetched_at: num(get(value, "fetched_at")?)? as i64,
        usage: Usage {
            five_hour: get(value, "five_hour").and_then(window),
            seven_day: get(value, "seven_day").and_then(window),
        },
    })
}

fn window(value: &JsonValue) -> Option<Window> {
    Some(Window {
        used_pct: num(get(value, "used_pct")?)?,
        resets_at: get(value, "resets_at").and_then(num).map(|n| n as i64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let path = std::env::temp_dir().join(format!("iusage-cache-test-{}.json", std::process::id()));
        let stored = Stored {
            cache: [
                Some(Entry {
                    fetched_at: 1_790_000_000,
                    announced: [1, 2],
                    usage: Usage {
                        five_hour: Some(Window { used_pct: 17.5, resets_at: Some(1_790_875_800) }),
                        seven_day: Some(Window { used_pct: 64.0, resets_at: None }),
                    },
                }),
                None,
            ],
            blocked_until: [1_790_000_300, 0],
        };
        save(&path, &stored).unwrap();
        assert_eq!(load(&path), stored);

        let other = Stored {
            cache: [None, Some(Entry { fetched_at: 5, announced: [0, 0], usage: Usage::default() })],
            blocked_until: [0, 77],
        };
        save(&path, &other).unwrap();
        assert_eq!(load(&path), other);
        std::fs::remove_file(&path).unwrap();
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
        assert_eq!(stored.blocked_until, [0, 0]);
        std::fs::remove_file(&path).unwrap();
    }
}
