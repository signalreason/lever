use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Map, Value};

type DynError = Box<dyn Error + Send + Sync + 'static>;

#[derive(Debug, Clone)]
struct RateLimitEntry {
    ts: f64,
    model: String,
    tokens: i64,
}

fn now_epoch_seconds() -> Result<f64, DynError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("System clock is before UNIX_EPOCH: {}", err))?;
    Ok(duration.as_secs_f64())
}

pub fn rate_limit_settings(model: &str) -> (u64, u64) {
    match model {
        "gpt-5.1-codex-mini" => (200_000, 500),
        "gpt-5.1-codex" | "gpt-5.2-codex" => (500_000, 500),
        _ => (200_000, 500),
    }
}

pub fn estimate_prompt_tokens(path: &Path) -> u64 {
    let size = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    if size == 0 {
        return 1000;
    }
    let estimate = ((size as f64) / 4.0).ceil() as u64;
    std::cmp::max(1000, estimate)
}

pub fn rate_limit_sleep_seconds(
    rate_file: &Path,
    model: &str,
    window: Duration,
    tpm_limit: u64,
    rpm_limit: u64,
    estimated_tokens: u64,
) -> Result<u64, DynError> {
    let now = now_epoch_seconds()?;
    Ok(rate_limit_sleep_seconds_at(
        rate_file,
        model,
        window,
        tpm_limit,
        rpm_limit,
        estimated_tokens,
        now,
    ))
}

pub fn record_rate_usage(
    rate_file: &Path,
    model: &str,
    window: Duration,
    tokens: u64,
) -> Result<(), DynError> {
    let now = now_epoch_seconds()?;
    record_rate_usage_at(rate_file, model, window, tokens, now)
}

fn rate_limit_sleep_seconds_at(
    rate_file: &Path,
    model: &str,
    window: Duration,
    tpm_limit: u64,
    rpm_limit: u64,
    estimated_tokens: u64,
    now: f64,
) -> u64 {
    let window_secs = window.as_secs_f64();
    let mut requests = read_rate_limit_requests(rate_file);
    let mut recent: Vec<RateLimitEntry> = requests
        .drain(..)
        .filter(|entry| entry.model == model && is_recent(entry, now, window_secs))
        .collect();
    recent.sort_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap_or(std::cmp::Ordering::Equal));

    let mut sleep_for = 0.0_f64;

    if rpm_limit > 0 && recent.len() >= rpm_limit as usize {
        let idx = recent.len().saturating_sub(rpm_limit as usize);
        if let Some(entry) = recent.get(idx) {
            let expire_at = entry.ts + window_secs;
            sleep_for = sleep_for.max(expire_at - now);
        }
    }

    if tpm_limit > 0 {
        let used: i64 = recent.iter().map(|entry| entry.tokens).sum();
        let estimated_tokens = estimated_tokens as i64;
        let limit = tpm_limit as i64;
        if used + estimated_tokens > limit {
            let over = used + estimated_tokens - limit;
            let mut dropped = 0_i64;
            for entry in &recent {
                dropped += entry.tokens;
                let expire_at = entry.ts + window_secs;
                if dropped >= over {
                    sleep_for = sleep_for.max(expire_at - now);
                    break;
                }
            }
        }
    }

    let sleep_for = sleep_for.max(0.0);
    (sleep_for + 0.999).floor() as u64
}

fn record_rate_usage_at(
    rate_file: &Path,
    model: &str,
    window: Duration,
    tokens: u64,
    now: f64,
) -> Result<(), DynError> {
    let window_secs = window.as_secs_f64();
    let (mut payload, mut requests) = read_rate_limit_payload(rate_file);
    requests.retain(|entry| is_recent(entry, now, window_secs));
    requests.push(RateLimitEntry {
        ts: now,
        model: model.to_string(),
        tokens: tokens as i64,
    });

    let mut entries = Vec::with_capacity(requests.len());
    for entry in requests {
        entries.push(rate_limit_entry_value(&entry));
    }

    if let Value::Object(map) = &mut payload {
        map.insert("requests".to_string(), Value::Array(entries));
    } else {
        payload = json!({ "requests": entries });
    }

    if let Some(parent) = rate_file.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "Failed to create rate limit directory {}: {}",
                    parent.display(),
                    err
                )
            })?;
        }
    }

    let serialized = serde_json::to_string(&payload)?;
    fs::write(rate_file, serialized).map_err(|err| {
        format!(
            "Failed to write rate limit file {}: {}",
            rate_file.display(),
            err
        )
    })?;
    Ok(())
}

fn read_rate_limit_payload(rate_file: &Path) -> (Value, Vec<RateLimitEntry>) {
    if !rate_file.exists() {
        return (json!({ "requests": [] }), Vec::new());
    }

    let raw = match fs::read_to_string(rate_file) {
        Ok(contents) => contents,
        Err(_) => return (json!({ "requests": [] }), Vec::new()),
    };

    let payload: Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return (json!({ "requests": [] }), Vec::new()),
    };

    let requests = extract_requests(&payload);
    (payload, requests)
}

fn read_rate_limit_requests(rate_file: &Path) -> Vec<RateLimitEntry> {
    let (_, requests) = read_rate_limit_payload(rate_file);
    requests
}

fn extract_requests(payload: &Value) -> Vec<RateLimitEntry> {
    let requests_value = match payload.get("requests") {
        Some(Value::Array(items)) => items,
        _ => return Vec::new(),
    };

    let mut requests = Vec::with_capacity(requests_value.len());
    for item in requests_value {
        let object = match item.as_object() {
            Some(map) => map,
            None => continue,
        };
        let ts = value_to_f64(object.get("ts"));
        let tokens = value_to_i64(object.get("tokens"));
        let model = object
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        requests.push(RateLimitEntry { ts, model, tokens });
    }
    requests
}

fn rate_limit_entry_value(entry: &RateLimitEntry) -> Value {
    let mut map = Map::new();
    map.insert("ts".to_string(), Value::from(entry.ts));
    map.insert("model".to_string(), Value::from(entry.model.clone()));
    map.insert("tokens".to_string(), Value::from(entry.tokens));
    Value::Object(map)
}

fn value_to_f64(value: Option<&Value>) -> f64 {
    match value {
        Some(Value::Number(num)) => num.as_f64().unwrap_or(0.0),
        Some(Value::String(text)) => text.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn value_to_i64(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(num)) => num.as_i64().or_else(|| num.as_u64().map(|v| v as i64)).unwrap_or(0),
        Some(Value::String(text)) => text.parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

fn is_recent(entry: &RateLimitEntry, now: f64, window_secs: f64) -> bool {
    if !entry.ts.is_finite() {
        return false;
    }
    now - entry.ts < window_secs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("lever-rate-limit-{}-{}", name, nanos));
        path
    }

    #[test]
    fn sleep_respects_rpm_limit() {
        let rate_file = temp_path("rpm");
        let window = Duration::from_secs(60);
        let now = 1000.0;

        let payload = json!({
            "requests": [
                { "ts": 950.0, "model": "gpt-5.2-codex", "tokens": 10 },
                { "ts": 980.0, "model": "gpt-5.2-codex", "tokens": 20 },
                { "ts": 990.0, "model": "gpt-5.2-codex", "tokens": 30 }
            ]
        });

        fs::create_dir_all(rate_file.parent().unwrap()).unwrap();
        fs::write(&rate_file, serde_json::to_string(&payload).unwrap()).unwrap();

        let sleep_seconds = rate_limit_sleep_seconds_at(
            &rate_file,
            "gpt-5.2-codex",
            window,
            0,
            2,
            0,
            now,
        );

        assert_eq!(sleep_seconds, 40);
    }

    #[test]
    fn sleep_respects_tpm_limit() {
        let rate_file = temp_path("tpm");
        let window = Duration::from_secs(60);
        let now = 1000.0;

        let payload = json!({
            "requests": [
                { "ts": 950.0, "model": "gpt-5.2-codex", "tokens": 50 },
                { "ts": 980.0, "model": "gpt-5.2-codex", "tokens": 30 }
            ]
        });

        fs::create_dir_all(rate_file.parent().unwrap()).unwrap();
        fs::write(&rate_file, serde_json::to_string(&payload).unwrap()).unwrap();

        let sleep_seconds = rate_limit_sleep_seconds_at(
            &rate_file,
            "gpt-5.2-codex",
            window,
            100,
            0,
            40,
            now,
        );

        assert_eq!(sleep_seconds, 10);
    }

    #[test]
    fn record_rate_usage_prunes_old_entries() {
        let rate_file = temp_path("record");
        let window = Duration::from_secs(60);
        let now = 1000.0;

        let payload = json!({
            "requests": [
                { "ts": 800.0, "model": "gpt-5.2-codex", "tokens": 10 },
                { "ts": 990.0, "model": "gpt-5.1-codex", "tokens": 20 }
            ],
            "extra": "keep"
        });

        fs::create_dir_all(rate_file.parent().unwrap()).unwrap();
        fs::write(&rate_file, serde_json::to_string(&payload).unwrap()).unwrap();

        record_rate_usage_at(&rate_file, "gpt-5.2-codex", window, 5, now).unwrap();

        let written: Value =
            serde_json::from_str(&fs::read_to_string(&rate_file).unwrap()).unwrap();
        let requests = written.get("requests").and_then(Value::as_array).unwrap();
        assert_eq!(written.get("extra").and_then(Value::as_str), Some("keep"));
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].get("model").and_then(Value::as_str),
            Some("gpt-5.1-codex")
        );
        assert_eq!(
            requests[1].get("model").and_then(Value::as_str),
            Some("gpt-5.2-codex")
        );
    }

    #[test]
    fn public_helpers_smoke() {
        let rate_file = temp_path("public");
        let prompt_path = temp_path("prompt");
        fs::create_dir_all(prompt_path.parent().unwrap()).unwrap();
        fs::write(&prompt_path, "abc").unwrap();

        let (tpm, rpm) = rate_limit_settings("gpt-5.2-codex");
        assert_eq!(tpm, 500_000);
        assert_eq!(rpm, 500);
        assert!(estimate_prompt_tokens(&prompt_path) >= 1000);

        let sleep_seconds = rate_limit_sleep_seconds(
            &rate_file,
            "gpt-5.2-codex",
            Duration::from_secs(60),
            tpm,
            rpm,
            0,
        )
        .unwrap();
        assert_eq!(sleep_seconds, 0);

        record_rate_usage(&rate_file, "gpt-5.2-codex", Duration::from_secs(60), 25).unwrap();
        let written: Value =
            serde_json::from_str(&fs::read_to_string(&rate_file).unwrap()).unwrap();
        let requests = written.get("requests").and_then(Value::as_array).unwrap();
        assert_eq!(requests.len(), 1);
    }
}
