//! Single-binary SPA embed.
//!
//! The React+antd console is built ahead of time into [`crate`] sibling
//! directory `../spa/dist/` (fixed file names — see `spa/vite.config.js`,
//! no content hashes) and embedded at compile time via `include_str!` /
//! `include_bytes!`. Consequence: `cargo build` never needs node; the
//! binary serves the exact committed dist output.
//!
//! Routing contract: `/` returns the HTML shell, `/static/:name` serves the
//! whitelisted build outputs. Both paths are auth-exempt (see
//! `auth_sig_mw`) so the console loads before the token is entered; the
//! whitelist is deliberate — exempt means the route is reachable without a
//! signature, not that arbitrary files are served.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};

use crate::AppState;

const INDEX_HTML: &str = include_str!("../spa/dist/index.html");
const APP_JS: &[u8] = include_bytes!("../spa/dist/static/app.js");
const APP_CSS: &[u8] = include_bytes!("../spa/dist/static/app.css");

/// SPA shell: the compile-time-embedded `spa/dist/index.html`.
pub async fn index(State(_state): State<Arc<AppState>>) -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// Whitelisted static build outputs (`/static/:name`). Anything outside the
/// fixed dist contract is 404 — no filesystem access, no traversal.
pub async fn static_asset(Path(name): Path<String>) -> Response {
    match name.as_str() {
        "app.js" => asset_response(APP_JS, "application/javascript"),
        "app.css" => asset_response(APP_CSS, "text/css"),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Pure responder: static bytes with a fixed content type.
fn asset_response(bytes: &'static [u8], content_type: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Bytes;

    async fn body(resp: Response) -> Bytes {
        axum::body::to_bytes(resp.into_body(), 4 << 20)
            .await
            .expect("body must read")
    }

    /// The bundle must actually mount the React app. A shell without a mounted
    /// bundle renders an empty #root in every browser — the kind of break no
    /// byte-level serve test catches (found by real-browser acceptance: main.jsx
    /// once shipped without its createRoot call while every other gate stayed
    /// green). Guard the contract at the embedded-bytes level.
    #[test]
    fn bundle_bootstraps_the_react_root() {
        assert!(INDEX_HTML.contains(r#"id="root""#), "shell must carry the #root mount node");
        let js = String::from_utf8_lossy(APP_JS);
        assert!(js.contains("createRoot"), "bundle must create a React root");
        assert!(
            js.contains(r#"getElementById("root")"#) || js.contains("getElementById('root')"),
            "bundle must mount into #root"
        );
    }

    /// The whitelist serves exactly the fixed build outputs, with the right
    /// content types; every other name is 404 (exempt ≠ anything goes).
    #[tokio::test]
    async fn static_whitelist_serves_fixed_build_outputs() {
        let js = static_asset(Path("app.js".to_string())).await;
        assert_eq!(js.status(), StatusCode::OK);
        assert_eq!(
            js.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/javascript")
        );
        assert!(!body(js).await.is_empty(), "app.js must be non-empty");

        let css = static_asset(Path("app.css".to_string())).await;
        assert_eq!(css.status(), StatusCode::OK);
        assert_eq!(
            css.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/css")
        );
        assert!(!body(css).await.is_empty(), "app.css must be non-empty");

        for name in ["nope.js", "index.html"] {
            let miss = static_asset(Path(name.to_string())).await;
            assert_eq!(miss.status(), StatusCode::NOT_FOUND, "{name} must 404");
        }
    }

    /// Pure scan: every `static/NAME` reference in the shell (chars up to the
    /// closing quote after each `static/` occurrence).
    fn static_refs(shell: &'static str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = shell;
        while let Some(pos) = rest.find("static/") {
            let after = &rest[pos + "static/".len()..];
            let name: String = after.chars().take_while(|c| *c != '"').collect();
            rest = after;
            if !name.is_empty() {
                names.push(name);
            }
        }
        names
    }

    /// Dist contract pin: the shell must never reference an asset the
    /// whitelist does not serve.
    #[tokio::test]
    async fn shell_references_resolve_through_the_whitelist() {
        let refs = static_refs(INDEX_HTML);
        assert!(
            !refs.is_empty(),
            "shell must reference at least one static asset"
        );
        for name in refs {
            let resp = static_asset(Path(name.clone())).await;
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "shell references static/{name} but the whitelist does not serve it"
            );
        }
    }

    /// Single binary, zero network: no external script/style/CDN references.
    #[test]
    fn shell_has_no_external_references() {
        assert!(!INDEX_HTML.contains("src=\"http"), "no external scripts");
        assert!(!INDEX_HTML.contains("href=\"http"), "no external styles");
        assert!(!INDEX_HTML.contains("//cdn"), "no CDN references");
    }

    /// The shell is the console entry: React mounts into #root, title names
    /// the product.
    #[test]
    fn shell_is_the_console_entry() {
        assert!(INDEX_HTML.contains("<div id=\"root\">"));
        assert!(INDEX_HTML.contains("Opencoder"));
    }
}
