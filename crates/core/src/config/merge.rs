use super::Config;
use super::HttpHeader;

/// `true` if `root` (a parsed config file) carries any of the editable
/// top-level or nested keys the `/model` menu can write.
pub(super) fn has_editable_key(root: &serde_json::Value) -> bool {
    let obj = match root.as_object() {
        Some(o) => o,
        None => return false,
    };
    if obj.contains_key("model")
        || obj.contains_key("small_model")
        || obj.contains_key("max_tokens")
        || obj.contains_key("reasoning_effort")
        || obj.contains_key("interleaved_thinking")
        || obj.contains_key("context_limit")
        || obj.contains_key("fps")
        || obj.contains_key("theme")
    {
        return true;
    }
    if obj
        .get("provider")
        .and_then(|v| v.as_object())
        .is_some_and(|p| p.contains_key("base_url") || p.contains_key("api_key"))
    {
        return true;
    }
    if obj
        .get("providers")
        .and_then(|v| v.as_object())
        .is_some_and(|p| !p.is_empty())
    {
        return true;
    }
    if obj
        .get("compaction")
        .and_then(|v| v.as_object())
        .is_some_and(|c| c.contains_key("context_threshold") || c.contains_key("auto"))
    {
        return true;
    }
    if obj
        .get("network")
        .and_then(|v| v.as_object())
        .is_some_and(|n| n.contains_key("proxy"))
    {
        return true;
    }
    if obj
        .get("capabilities")
        .and_then(|v| v.as_object())
        .is_some_and(|c| {
            c.contains_key("browser")
                || c.contains_key("computer_use")
                || c.contains_key("tools_subagent")
        })
    {
        return true;
    }
    if obj
        .get("autopilot")
        .and_then(|v| v.as_object())
        .is_some_and(|a| {
            a.contains_key("enabled")
                || a.contains_key("max_iterations")
                || a.contains_key("verify_retries")
        })
    {
        return true;
    }
    false
}

/// Recursive JSON object merge: `patch` wins; nested objects are merged
/// key-by-key rather than replaced wholesale, so editing `compaction.context_threshold`
/// preserves a sibling `tail_turns`.
pub(super) fn merge_json(dst: &mut serde_json::Value, patch: &serde_json::Value) {
    use serde_json::Value;
    match (dst, patch) {
        (Value::Object(d), Value::Object(p)) => {
            for (k, pv) in p {
                match (d.get_mut(k), pv) {
                    (Some(Value::Object(_)), Value::Object(_)) => {
                        if let Some(child) = d.get_mut(k) {
                            merge_json(child, pv);
                        }
                    }
                    (_, Value::Null) => {
                        d.remove(k);
                    }
                    _ => {
                        d.insert(k.clone(), pv.clone());
                    }
                }
            }
        }
        (d, p) => {
            *d = p.clone();
        }
    }
}

/// Apply a parsed config JSON `value` onto `cfg`, field by field. Only the
/// keys present in `value` are overwritten; everything else is left as-is.
pub(super) fn merge_into(cfg: &mut Config, value: serde_json::Value) {
    if let Some(obj) = value.as_object() {
        if let Some(model) = obj.get("model").and_then(|v| v.as_str()) {
            cfg.model = model.to_string();
        }
        if let Some(small) = obj.get("small_model").and_then(|v| v.as_str()) {
            cfg.small_model = Some(small.to_string());
        }
        if let Some(cl) = obj.get("context_limit").and_then(|v| v.as_u64()) {
            cfg.context_limit = Some(cl);
        }
        if let Some(mt) = obj.get("max_tokens").and_then(|v| v.as_u64()) {
            cfg.max_tokens = Some(mt);
        }
        if let Some(re) = obj.get("reasoning_effort").and_then(|v| v.as_str()) {
            let trimmed = re.trim();
            if trimmed.is_empty() {
                cfg.reasoning_effort = None;
            } else {
                cfg.reasoning_effort = Some(trimmed.to_string());
            }
        }
        if let Some(it) = obj.get("interleaved_thinking").and_then(|v| v.as_bool()) {
            cfg.interleaved_thinking = Some(it);
        }
        if let Some(v) = obj.get("cache_salt").and_then(|v| v.as_bool()) {
            cfg.cache_salt = Some(v);
        }
        if let Some(v) = obj.get("enable_tmux_session").and_then(|v| v.as_bool()) {
            cfg.enable_tmux_session = Some(v);
        }
        if let Some(fps) = obj.get("fps").and_then(|v| v.as_u64()) {
            cfg.fps = Some(fps.clamp(1, 30) as u32);
        }
        if let Some(t) = obj.get("theme").and_then(|v| v.as_str()) {
            cfg.theme = t.to_string();
        }
        if let Some(p) = obj.get("provider").and_then(|v| v.as_object()) {
            if let Some(b) = p.get("base_url").and_then(|v| v.as_str()) {
                cfg.provider.base_url = b.to_string();
            }
            if let Some(k) = p.get("api_key").and_then(|v| v.as_str()) {
                cfg.provider.api_key = Some(super::resolve_env(k));
            }
        }
        if let Some(providers) = obj.get("providers").and_then(|v| v.as_object()) {
            for (name, pv) in providers {
                if let Some(pcfg) = pv.as_object() {
                    let entry = cfg.providers.entry(name.clone()).or_default();
                    if let Some(b) = pcfg.get("base_url").and_then(|v| v.as_str()) {
                        entry.base_url = b.to_string();
                    }
                    if let Some(k) = pcfg.get("api_key").and_then(|v| v.as_str()) {
                        entry.api_key = Some(super::resolve_env(k));
                    }
                    if let Some(m) = pcfg.get("model").and_then(|v| v.as_str()) {
                        entry.model = Some(m.to_string());
                    }
                    if let Some(hs) = pcfg.get("headers").and_then(|v| v.as_array()) {
                        // Append rather than replace: a project file's headers
                        // extend the global set instead of clobbering it (other
                        // sub-fields above are merged field-by-field for the
                        // same reason).
                        entry.headers.extend(hs.iter().filter_map(|h| {
                            let name = h.get("name")?.as_str()?.to_string();
                            let value = h.get("value")?.as_str()?.to_string();
                            Some(HttpHeader { name, value })
                        }));
                    }
                }
            }
        }
        if let Some(c) = obj.get("compaction").and_then(|v| v.as_object()) {
            if let Some(v) = c.get("auto").and_then(|v| v.as_bool()) {
                cfg.compaction.auto = v;
            }
            if let Some(v) = c.get("context_threshold").and_then(|v| v.as_u64()) {
                cfg.compaction.context_threshold = v;
            }
            if let Some(v) = c.get("tail_turns").and_then(|v| v.as_u64()) {
                cfg.compaction.tail_turns = v.min(u32::MAX as u64) as u32;
            }
            if let Some(v) = c.get("reserved").and_then(|v| v.as_u64()) {
                cfg.compaction.reserved = v;
            }
            if let Some(v) = c.get("buffer").and_then(|v| v.as_u64()) {
                cfg.compaction.buffer = Some(v);
            }
        }
        if let Some(a) = obj.get("agent").and_then(|v| v.as_object()) {
            if let Some(d) = a.get("default").and_then(|v| v.as_str()) {
                cfg.agent.default = d.to_string();
            }
        }
        if let Some(n) = obj.get("network").and_then(|v| v.as_object()) {
            if let Some(p) = n.get("proxy").and_then(|v| v.as_str()) {
                let t = p.trim();
                cfg.network.proxy = if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                };
            }
        }
        if let Some(c) = obj.get("capabilities").and_then(|v| v.as_object()) {
            if let Some(b) = c.get("browser").and_then(|v| v.as_bool()) {
                cfg.capabilities.browser = b;
            }
            if let Some(b) = c.get("computer_use").and_then(|v| v.as_bool()) {
                cfg.capabilities.computer_use = b;
            }
            if let Some(b) = c.get("tools_subagent").and_then(|v| v.as_bool()) {
                cfg.capabilities.tools_subagent = b;
            }
        }
        if let Some(v) = obj.get("stream_idle_timeout_secs").and_then(|v| v.as_u64()) {
            cfg.stream_idle_timeout_secs = Some(v);
        }
        if let Some(v) = obj.get("task_timeout_secs").and_then(|v| v.as_u64()) {
            cfg.task_timeout_secs = Some(v);
        }
        if let Some(v) = obj.get("replay_timeout_secs").and_then(|v| v.as_u64()) {
            cfg.replay_timeout_secs = Some(v);
        }
        if let Some(a) = obj.get("autopilot").and_then(|v| v.as_object()) {
            super::autopilot::merge(&mut cfg.autopilot, a);
        }
        if let Some(o) = obj.get("output_streamline").and_then(|v| v.as_object()) {
            if let Some(b) = o.get("enabled").and_then(|v| v.as_bool()) {
                cfg.output_streamline.enabled = b;
            }
            if let Some(b) = o.get("trim_trailing").and_then(|v| v.as_bool()) {
                cfg.output_streamline.trim_trailing = b;
            }
            if let Some(b) = o.get("collapse_blank_lines").and_then(|v| v.as_bool()) {
                cfg.output_streamline.collapse_blank_lines = b;
            }
            if let Some(b) = o.get("trim_outer").and_then(|v| v.as_bool()) {
                cfg.output_streamline.trim_outer = b;
            }
            if let Some(b) = o.get("collapse_inline_ws").and_then(|v| v.as_bool()) {
                cfg.output_streamline.collapse_inline_ws = b;
            }
        }
        if let Some(t) = obj.get("tool_guard").and_then(|v| v.as_object()) {
            if let Some(v) = t.get("max_consecutive_failures").and_then(|v| v.as_u64()) {
                cfg.tool_guard.max_consecutive_failures = v.min(u32::MAX as u64) as u32;
            }
            if let Some(v) = t.get("backoff_base_ms").and_then(|v| v.as_u64()) {
                cfg.tool_guard.backoff_base_ms = v;
            }
            if let Some(v) = t.get("backoff_max_ms").and_then(|v| v.as_u64()) {
                cfg.tool_guard.backoff_max_ms = v;
            }
        }
        if let Some(v) = obj.get("subagent_drain_secs").and_then(|v| v.as_u64()) {
            cfg.subagent_drain_secs = Some(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::merge_into;
    use crate::config::Config;

    /// Regression: `tool_guard.max_consecutive_failures` is a `u32` but the
    /// merged JSON value is read as `u64`. An unclamped `v as u32` cast would
    /// silently truncate `u32::MAX + 1` (= 4_294_967_296) to `0`, which per
    /// `ToolGuardConfig` semantics means "guard disabled". The merge must clamp
    /// to `u32::MAX` instead.
    #[test]
    fn tool_guard_max_consecutive_failures_clamps_overflow() {
        let mut cfg = Config::default();
        let value = serde_json::json!({
            "tool_guard": {
                "max_consecutive_failures": 4_294_967_296u64,
            }
        });
        merge_into(&mut cfg, value);
        assert_eq!(cfg.tool_guard.max_consecutive_failures, u32::MAX);
    }
}
