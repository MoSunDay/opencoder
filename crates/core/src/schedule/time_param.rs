//! `{{now...}}` time-parameter rendering for schedule params (pure).
//!
//! String values may embed `{{now[±N<unit>][:format]}}` tokens which are
//! resolved at fire time against the planned fire instant:
//!
//! - `{{now}}` — RFC3339 (UTC, second precision)
//! - `{{now-1d:%Y-%m-%d}}` — one day earlier, custom chrono format
//! - `{{now+30m:%H:%M}}` / `{{now:unix_ms}}` — offsets and epoch aliases
//!
//! Units: `s` `m` `h` `d` `w`. Unknown tokens, malformed offsets, unknown
//! units and invalid format strings are errors (validation dry-runs every
//! schedule so typos surface at config time, not at 3am).

use chrono::{DateTime, FixedOffset};
use serde_json::Value;

/// Render every `{{...}}` token in string values (recursing through objects
/// and arrays); non-string values pass through untouched.
pub fn render_params(value: &Value, now: DateTime<FixedOffset>) -> Result<Value, String> {
    match value {
        Value::String(raw) => render_str(raw, now).map(Value::String),
        Value::Array(items) => items
            .iter()
            .map(|item| render_params(item, now))
            .collect::<Result<Vec<_>, String>>()
            .map(Value::Array),
        Value::Object(entries) => {
            let mut out = serde_json::Map::with_capacity(entries.len());
            for (key, item) in entries {
                out.insert(key.clone(), render_params(item, now)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

fn render_str(raw: &str, now: DateTime<FixedOffset>) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail.find("}}").ok_or_else(|| {
            format!(
                "time parameter without closing `}}}}` in {raw:?} (start {:#?})",
                &rest[start..]
            )
        })?;
        out.push_str(&eval_token(tail[..end].trim(), now, raw)?);
        rest = &tail[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn eval_token(token: &str, now: DateTime<FixedOffset>, source: &str) -> Result<String, String> {
    let (expr, format) = match token.split_once(':') {
        Some((expr, format)) => (expr.trim(), Some(format.trim())),
        None => (token, None),
    };
    let shifted = apply_offset(expr, now)?;
    Ok(match format {
        None => shifted.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        Some("") => return Err("empty format string in time parameter".to_string()),
        Some("unix") => shifted.timestamp().to_string(),
        Some("unix_ms") => shifted.timestamp_millis().to_string(),
        Some(format) => render_format(shifted, format, token, source)?,
    })
}

fn apply_offset(expr: &str, now: DateTime<FixedOffset>) -> Result<DateTime<FixedOffset>, String> {
    if expr == "now" {
        return Ok(now);
    }
    let offset = expr
        .strip_prefix("now")
        .ok_or_else(|| format!("unknown time parameter {expr:?} (expected `now[±N<unit>]`)"))?;
    let (sign, magnitude) = match offset.as_bytes().first() {
        Some(b'+') => (1i64, &offset[1..]),
        Some(b'-') => (-1i64, &offset[1..]),
        _ => {
            return Err(format!(
                "unknown time parameter {expr:?} (expected `now` or `now±N<unit>`)"
            ))
        }
    };
    let digits_end = magnitude
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(magnitude.len());
    if digits_end == 0 {
        return Err(format!("time parameter {expr:?} is missing a step count"));
    }
    let count: i64 = magnitude[..digits_end]
        .parse()
        .map_err(|_| format!("time parameter {expr:?} has a step count out of range"))?;
    let seconds = match &magnitude[digits_end..] {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        unit => {
            return Err(format!(
                "unknown time unit {unit:?} in {expr:?} (use s/m/h/d/w)"
            ))
        }
    };
    let delta = chrono::Duration::seconds(sign * count * seconds);
    Ok(now + delta)
}

fn render_format(
    value: DateTime<FixedOffset>,
    format: &str,
    token: &str,
    source: &str,
) -> Result<String, String> {
    if chrono::format::StrftimeItems::new(format)
        .any(|item| matches!(item, chrono::format::Item::Error))
    {
        return Err(format!(
            "invalid format string {format:?} in time parameter {token:?} of {source:?}"
        ));
    }
    Ok(value.format(format).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn now() -> DateTime<FixedOffset> {
        Utc.with_ymd_and_hms(2026, 9, 17, 2, 5, 30)
            .unwrap()
            .fixed_offset()
    }

    fn rendered(value: Value) -> String {
        render_params(&value, now())
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn default_format_is_rfc3339_utc() {
        assert_eq!(rendered(json!("{{now}}")), "2026-09-17T02:05:30Z");
    }

    #[test]
    fn offsets_and_units_resolve() {
        assert_eq!(rendered(json!("{{now-1d:%Y-%m-%d}}")), "2026-09-16");
        assert_eq!(rendered(json!("{{now+30m:%H:%M}}")), "02:35");
        assert_eq!(rendered(json!("{{now-90m:%H:%M}}")), "00:35");
        assert_eq!(rendered(json!("{{now+2w:%F}}")), "2026-10-01");
        assert_eq!(rendered(json!("{{now-5s:%S}}")), "25");
        let unix: i64 = rendered(json!("{{now:unix}}")).parse().unwrap();
        assert_eq!(
            rendered(json!("{{now-1h:unix}}")),
            (unix - 3600).to_string()
        );
    }

    #[test]
    fn epoch_aliases_render_as_numbers_in_strings() {
        // 2026-09-17T02:05:30Z
        assert_eq!(rendered(json!("{{now:unix_ms}}")), "1789610730000");
        assert_eq!(rendered(json!("{{now:unix}}")), "1789610730");
    }

    #[test]
    fn tokens_embed_inside_larger_strings_and_multiple_per_string() {
        let value = json!({"title": "日报 {{now-1d:%Y-%m-%d}}", "nested": {"p": "统计 {{now}} 至 {{now+1h:unix_ms}}"}});
        let out = render_params(&value, now()).unwrap();
        assert_eq!(out["title"], "日报 2026-09-16");
        assert_eq!(
            out["nested"]["p"],
            "统计 2026-09-17T02:05:30Z 至 1789614330000"
        );
    }

    #[test]
    fn non_string_values_pass_through() {
        let value = json!({"n": 3, "b": true, "x": null, "a": [1, "keep"]});
        assert_eq!(render_params(&value, now()).unwrap(), value);
    }

    #[test]
    fn invalid_tokens_are_errors() {
        for bad in [
            "{{nope}}",
            "{{now+1x}}",
            "{{now-}}",
            "{{now:}}",
            "{{now:%Q}}",
            "{{now",
        ] {
            assert!(
                render_params(&json!(bad), now()).is_err(),
                "{bad} must fail"
            );
        }
    }

    #[test]
    fn renders_in_the_provided_timezone() {
        let tz = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        let local = Utc
            .with_ymd_and_hms(2026, 9, 17, 19, 0, 0)
            .unwrap()
            .with_timezone(&tz);
        let out = render_params(&json!("{{now:%F %T}}"), local).unwrap();
        assert_eq!(out, "2026-09-18 03:00:00");
    }
}
