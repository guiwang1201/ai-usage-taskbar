//! Where each provider keeps its credentials, which endpoint reports usage, and how to
//! read the answer. Read-only: credential files are never written and tokens never refreshed.

use std::path::{Path, PathBuf};

use tinyjson::JsonValue;

use crate::json::{self, get, num, text};
use crate::model::{FetchError, Provider, Usage, Window};
use crate::timefmt::parse_iso8601;

const USER_AGENT: &str = concat!("iusage/", env!("CARGO_PKG_VERSION"));
/// Used when no installed Claude Code version can be found.
const FALLBACK_CLAUDE_CODE_VERSION: &str = "2.1.284";

/// Where the providers' own software keeps things on this machine.
pub struct Env {
    pub home: PathBuf,
    /// Folder in which the Claude desktop app keeps one sub-folder per Claude Code version.
    pub claude_code_dir: PathBuf,
}

/// An HTTPS GET, described without any platform types.
pub struct Request {
    pub host: &'static str,
    pub path: &'static str,
    pub user_agent: String,
    pub headers: Vec<(&'static str, String)>,
}

pub struct Response {
    pub status: u16,
    pub body: String,
    /// The Retry-After header in seconds, if present.
    pub retry_after: Option<u32>,
}

/// Platform HTTP client.
pub type HttpGet = fn(&Request) -> Result<Response, String>;

pub fn fetch(provider: Provider, env: &Env, now: i64, http: HttpGet) -> Result<Usage, FetchError> {
    let request = build_request(provider, env, now)?;
    let response = http(&request).map_err(FetchError::Network)?;
    match response.status {
        200 => parse(provider, &response.body),
        401 | 403 => Err(FetchError::Unauthorized),
        429 => Err(FetchError::RateLimited(response.retry_after)),
        other => Err(FetchError::Http(other)),
    }
}

pub fn parse(provider: Provider, body: &str) -> Result<Usage, FetchError> {
    let root = json::parse(body).ok_or(FetchError::Parse)?;
    match provider {
        Provider::Claude => parse_claude(&root),
        Provider::Codex => parse_codex(&root),
    }
}

fn config_dir(env_var: &str, home: &Path, default: &str) -> PathBuf {
    match std::env::var_os(env_var) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home.join(default),
    }
}

fn read_json(path: &Path) -> Result<JsonValue, FetchError> {
    let text = std::fs::read_to_string(path).map_err(|_| FetchError::NoCredentials)?;
    json::parse(&text).ok_or(FetchError::NoCredentials)
}

/// The usage endpoint exists for Claude Code, and other tools that read it all identify
/// as Claude Code; callers that do not are believed to be rate limited sooner. That has
/// not been measured here. The request therefore names the Claude Code version that is
/// installed on this machine.
fn claude_user_agent(claude_code_dir: &Path) -> String {
    let version = installed_claude_code(claude_code_dir).unwrap_or_else(|| FALLBACK_CLAUDE_CODE_VERSION.to_string());
    format!("claude-code/{version}")
}

/// The highest `major.minor.patch` among the folder names in `dir`.
fn installed_claude_code(dir: &Path) -> Option<String> {
    let parse = |name: &str| -> Option<[u32; 3]> {
        let mut parts = name.split('.');
        let version = [parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?];
        parts.next().is_none().then_some(version)
    };
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| parse(entry.ok()?.file_name().to_str()?))
        .max()
        .map(|[major, minor, patch]| format!("{major}.{minor}.{patch}"))
}

fn build_request(provider: Provider, env: &Env, now: i64) -> Result<Request, FetchError> {
    let home = env.home.as_path();
    match provider {
        Provider::Claude => {
            let file = config_dir("CLAUDE_CONFIG_DIR", home, ".claude").join(".credentials.json");
            let root = read_json(&file)?;
            let oauth = get(&root, "claudeAiOauth").ok_or(FetchError::NoCredentials)?;
            let token = get(oauth, "accessToken").and_then(text).ok_or(FetchError::NoCredentials)?;
            // expiresAt is in milliseconds. An expired token is not sent at all: the
            // desktop app rewrites the file with a fresh one the next time it runs.
            if let Some(expires_ms) = get(oauth, "expiresAt").and_then(num) {
                if (expires_ms / 1000.0) as i64 <= now {
                    return Err(FetchError::Expired);
                }
            }
            Ok(Request {
                host: "api.anthropic.com",
                path: "/api/oauth/usage",
                user_agent: claude_user_agent(&env.claude_code_dir),
                headers: vec![
                    ("Authorization", format!("Bearer {token}")),
                    ("anthropic-beta", "oauth-2025-04-20".to_string()),
                    ("Accept", "application/json".to_string()),
                ],
            })
        }
        Provider::Codex => {
            let file = config_dir("CODEX_HOME", home, ".codex").join("auth.json");
            let root = read_json(&file)?;
            let tokens = get(&root, "tokens").ok_or(FetchError::NoCredentials)?;
            let token = get(tokens, "access_token").and_then(text).ok_or(FetchError::NoCredentials)?;
            let mut headers = vec![
                ("Authorization", format!("Bearer {token}")),
                ("Accept", "application/json".to_string()),
            ];
            if let Some(account) = get(tokens, "account_id").and_then(text) {
                headers.push(("ChatGPT-Account-Id", account.to_string()));
            }
            Ok(Request {
                host: "chatgpt.com",
                path: "/backend-api/wham/usage",
                user_agent: USER_AGENT.to_string(),
                headers,
            })
        }
    }
}

fn parse_claude(root: &JsonValue) -> Result<Usage, FetchError> {
    if !matches!(root, JsonValue::Object(_)) {
        return Err(FetchError::Parse);
    }
    let window = |key: &str| -> Option<Window> {
        let w = get(root, key)?;
        Some(Window {
            used_pct: get(w, "utilization").and_then(num)?,
            resets_at: get(w, "resets_at").and_then(text).and_then(parse_iso8601),
        })
    };
    Ok(Usage { five_hour: window("five_hour"), seven_day: window("seven_day") })
}

fn parse_codex(root: &JsonValue) -> Result<Usage, FetchError> {
    let limits = get(root, "rate_limit").ok_or(FetchError::Parse)?;
    if !matches!(limits, JsonValue::Object(_)) {
        return Err(FetchError::Parse);
    }
    let mut usage = Usage::default();
    for (key, is_primary) in [("primary_window", true), ("secondary_window", false)] {
        let Some(w) = get(limits, key) else { continue };
        let Some(used_pct) = get(w, "used_percent").and_then(num) else { continue };
        let window = Window {
            used_pct,
            resets_at: get(w, "reset_at").and_then(num).map(|n| n as i64),
        };
        // Tell the windows apart by their length; fall back to position if it is missing.
        let short = match get(w, "limit_window_seconds").and_then(num) {
            Some(secs) if secs > 0.0 => secs <= 86_400.0,
            _ => is_primary,
        };
        if short {
            usage.five_hour = Some(window);
        } else {
            usage.seven_day = Some(window);
        }
    }
    Ok(usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from real responses captured in phase 0.
    const CLAUDE: &str = r#"{"five_hour":{"utilization":4.0,"resets_at":"2026-10-01T17:30:00.472092+00:00","limit_dollars":null},
        "seven_day":{"utilization":62.0,"resets_at":"2026-10-02T02:00:00.472116+00:00"},
        "seven_day_opus":null,"iguana_necktie":{"utilization":0.0,"resets_at":"2026-11-05T07:59:00+00:00"},
        "extra_usage":{"is_enabled":false,"monthly_limit":null}}"#;

    const CODEX: &str = r#"{"plan_type":"plus","rate_limit":{"allowed":true,"limit_reached":false,
        "primary_window":{"used_percent":0,"limit_window_seconds":18000,"reset_after_seconds":18000,"reset_at":1790876671},
        "secondary_window":{"used_percent":8,"limit_window_seconds":604800,"reset_after_seconds":250205,"reset_at":1791108875}},
        "code_review_rate_limit":null,"credits":{"has_credits":false}}"#;

    #[test]
    fn claude_requests_carry_the_installed_claude_code_version() {
        let dir = std::env::temp_dir().join(format!("iusage-cc-test-{}", std::process::id()));
        assert_eq!(claude_user_agent(&dir), format!("claude-code/{FALLBACK_CLAUDE_CODE_VERSION}"));
        for name in ["2.1.99", "2.1.284", "2.0.300", "notes", "3.x.1", "2.1.284.1"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        assert_eq!(claude_user_agent(&dir), "claude-code/2.1.284");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_claude_usage() {
        let usage = parse(Provider::Claude, CLAUDE).unwrap();
        assert_eq!(usage.five_hour, Some(Window { used_pct: 4.0, resets_at: Some(1_790_875_800) }));
        assert_eq!(usage.seven_day, Some(Window { used_pct: 62.0, resets_at: Some(1_790_906_400) }));
    }

    #[test]
    fn claude_window_without_active_session() {
        let usage = parse(Provider::Claude, r#"{"five_hour":null,"seven_day":{"utilization":10,"resets_at":null}}"#).unwrap();
        assert_eq!(usage.five_hour, None);
        assert_eq!(usage.seven_day, Some(Window { used_pct: 10.0, resets_at: None }));
    }

    #[test]
    fn parses_codex_usage() {
        let usage = parse(Provider::Codex, CODEX).unwrap();
        assert_eq!(usage.five_hour, Some(Window { used_pct: 0.0, resets_at: Some(1_790_876_671) }));
        assert_eq!(usage.seven_day, Some(Window { used_pct: 8.0, resets_at: Some(1_791_108_875) }));
    }

    #[test]
    fn codex_windows_are_identified_by_length_not_position() {
        let swapped = r#"{"rate_limit":{
            "primary_window":{"used_percent":8,"limit_window_seconds":604800,"reset_at":2},
            "secondary_window":{"used_percent":3,"limit_window_seconds":18000,"reset_at":1}}}"#;
        let usage = parse(Provider::Codex, swapped).unwrap();
        assert_eq!(usage.five_hour.unwrap().used_pct, 3.0);
        assert_eq!(usage.seven_day.unwrap().used_pct, 8.0);
    }

    #[test]
    fn rejects_unexpected_bodies() {
        assert_eq!(parse(Provider::Claude, "<html>blocked</html>"), Err(FetchError::Parse));
        assert_eq!(parse(Provider::Claude, "[]"), Err(FetchError::Parse));
        assert_eq!(parse(Provider::Codex, r#"{"rate_limit":null}"#), Err(FetchError::Parse));
        assert_eq!(parse(Provider::Claude, &"[".repeat(200_000)), Err(FetchError::Parse));
    }
}
