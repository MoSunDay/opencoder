use std::sync::LazyLock;

use axum::extract::State;
use axum::response::Html;
use std::sync::Arc;

use crate::AppState;

/// Single-binary SPA: every frontend asset is embedded via `include_str!`
/// and inlined into one HTML document computed exactly once at first use.
/// No static file serving, no external requests.
pub async fn index(State(_state): State<Arc<AppState>>) -> Html<&'static str> {
    Html(&MANAGER_HTML)
}

const SHELL: &str = include_str!("assets/index.html");
const CSS: &str = include_str!("assets/styles.css");

// Classic scripts in dependency order: state/helpers first, controller last.
// api.js (token + fetch) is required by everything; sse.js before chat.js
// (which registers handlers via onSSE); composer/settings/questions/queue
// are wired at load time but only call each other at runtime.
const JS_API: &str = include_str!("assets/api.js");
const JS_SSE: &str = include_str!("assets/sse.js");
const JS_SESSIONS: &str = include_str!("assets/sessions.js");
const JS_CHAT: &str = include_str!("assets/chat.js");
const JS_COMPOSER: &str = include_str!("assets/composer.js");
const JS_QUESTIONS: &str = include_str!("assets/questions.js");
const JS_QUEUE: &str = include_str!("assets/queue_panel.js");
const JS_SUBAGENTS: &str = include_str!("assets/subagent_view.js");
const JS_BG: &str = include_str!("assets/bg_panel.js");
const JS_NODES: &str = include_str!("assets/nodes_panel.js");
const JS_SETTINGS: &str = include_str!("assets/settings.js");

/// Wrap one script body in an inline `<script>` tag (classic script, no
/// modules/bundlers — single-binary inline only).
fn inline_script(js: &str) -> String {
    format!("<script>\n{js}\n</script>")
}

static MANAGER_HTML: LazyLock<String> = LazyLock::new(|| {
    let mut html = SHELL.replace("<!--STYLES-->", &format!("<style>\n{CSS}\n</style>"));
    let scripts: &[(&str, &str)] = &[
        ("<!--JS_API-->", JS_API),
        ("<!--JS_SSE-->", JS_SSE),
        ("<!--JS_SESSIONS-->", JS_SESSIONS),
        ("<!--JS_CHAT-->", JS_CHAT),
        ("<!--JS_COMPOSER-->", JS_COMPOSER),
        ("<!--JS_QUESTIONS-->", JS_QUESTIONS),
        ("<!--JS_QUEUE-->", JS_QUEUE),
        ("<!--JS_SUBAGENTS-->", JS_SUBAGENTS),
        ("<!--JS_BG-->", JS_BG),
        ("<!--JS_NODES-->", JS_NODES),
        ("<!--JS_SETTINGS-->", JS_SETTINGS),
    ];
    for (marker, js) in scripts {
        html = html.replace(marker, &inline_script(js));
    }
    html
});

#[cfg(test)]
mod tests {
    use super::*;

    /// Every marker must be consumed and its asset inlined exactly once.
    #[test]
    fn markers_replaced_and_assets_inlined() {
        let html = MANAGER_HTML.as_str();
        for marker in [
            "<!--STYLES-->",
            "<!--JS_API-->",
            "<!--JS_SSE-->",
            "<!--JS_SESSIONS-->",
            "<!--JS_CHAT-->",
            "<!--JS_COMPOSER-->",
            "<!--JS_QUESTIONS-->",
            "<!--JS_QUEUE-->",
            "<!--JS_NODES-->",
            "<!--JS_SETTINGS-->",
            // absorbed legacy markers must be gone entirely
            "<!--JS_RENDER-->",
            "<!--JS_APP-->",
        ] {
            assert!(!html.contains(marker), "marker {marker} still present");
        }
        assert!(html.contains("<style>"));
        assert!(html.contains("var(--bg)"));
        // new DOM skeletons the scripts bind to
        for skeleton in [
            "id=\"qpanel\"",
            "id=\"questions\"",
            "id=\"hero\"",
            "id=\"skill-pop\"",
            "id=\"settings-pop\"",
            "id=\"reconnect\"",
            "id=\"reconnect-fail\"",
            "id=\"qcount\"",
            "id=\"nodes-panel\"",
            "id=\"nodes-live\"",
        ] {
            assert!(html.contains(skeleton), "missing skeleton {skeleton}");
        }
    }

    /// Each module's sentinel function must exist, and the scripts must be
    /// inlined in dependency order (api -> sse -> sessions -> chat ->
    /// composer -> questions -> queue_panel -> settings).
    #[test]
    fn script_sentinels_present_in_dependency_order() {
        let html = MANAGER_HTML.as_str();
        let sentinels: &[(&str, &str)] = &[
            ("api.js", "function apiGet"),
            ("sse.js", "function openStream"),
            ("sessions.js", "function refreshSessions"),
            ("chat.js", "function renderMessages"),
            ("composer.js", "async function send"),
            ("questions.js", "function pollQuestions"),
            ("queue_panel.js", "function refreshQueuePanel"),
            ("subagent_view.js", "function subagentViewClick"),
            ("bg_panel.js", "function refreshBgPanel"),
            ("nodes_panel.js", "function toggleNodesPanel"),
            ("settings.js", "function loadModels"),
        ];
        let mut prev = 0usize;
        for (name, sentinel) in sentinels {
            let pos = html.find(sentinel).unwrap_or_else(|| {
                panic!("{name} sentinel `{sentinel}` missing from inlined HTML")
            });
            assert!(
                pos > prev,
                "{name} ({sentinel}) must come after the previous script (dependency order)"
            );
            prev = pos;
        }
    }

    /// The restructure stays classic-script vanilla JS: no ES module syntax,
    /// no external/CDN references (single binary, no network at load).
    #[test]
    fn no_module_or_external_references() {
        let html = MANAGER_HTML.as_str();
        assert!(!html.contains("export "));
        assert!(!html.contains("import "));
        assert!(!html.contains("<script src="));
        assert!(!html.contains("http://") && !html.contains("https://"));
    }
}
