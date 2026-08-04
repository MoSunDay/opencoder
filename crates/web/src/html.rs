use axum::extract::State;
use axum::response::Html;
use std::sync::Arc;
use std::sync::LazyLock;

use crate::AppState;

/// Single-binary SPA: every frontend asset is embedded via `include_str!`
/// and inlined into one HTML document computed exactly once at first use.
/// No static file serving, no external requests.
pub async fn index(State(_state): State<Arc<AppState>>) -> Html<&'static str> {
    Html(&MANAGER_HTML)
}

const SHELL: &str = include_str!("assets/index.html");
const CSS: &str = include_str!("assets/styles.css");
const JS_RENDER: &str = include_str!("assets/render.js");
const JS_APP: &str = include_str!("assets/app.js");

static MANAGER_HTML: LazyLock<String> = LazyLock::new(|| {
    let with_css = SHELL.replace("<!--STYLES-->", &format!("<style>\n{CSS}\n</style>"));
    let with_render = with_css.replace("<!--JS_RENDER-->", &format!("<script>\n{JS_RENDER}\n</script>"));
    with_render.replace("<!--JS_APP-->", &format!("<script>\n{JS_APP}\n</script>"))
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_replaced_and_assets_inlined() {
        let html = MANAGER_HTML.as_str();
        assert!(!html.contains("<!--STYLES-->"));
        assert!(!html.contains("<!--JS_RENDER-->"));
        assert!(!html.contains("<!--JS_APP-->"));
        assert!(html.contains("<style>"));
        assert!(html.contains("var(--bg)"));
        assert!(html.contains("function openStream"));
        assert!(html.contains("async function send"));
        assert!(html.contains("newSession"));
    }

    #[test]
    fn scripts_rendered_before_app() {
        let html = MANAGER_HTML.as_str();
        let r = html.find("function openStream").unwrap();
        let a = html.find("async function send").unwrap();
        assert!(r < a, "render.js (state/helpers) must precede app.js (controller)");
    }
}
