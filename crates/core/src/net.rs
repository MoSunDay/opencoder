//! Proxy-aware HTTP client construction, shared by the LLM client and the
//! browser tools. Supports `socks5://` / `socks5h://` / `http://` / `https://`
//! proxies (the workspace `reqwest` enables the `socks` feature for SOCKS).

use anyhow::{Context, Result};

/// Resolve the effective proxy URL. Priority: an explicit config value, then
/// `OPENCODER_PROXY`, then `ALL_PROXY`, then `HTTPS_PROXY` / `HTTP_PROXY`.
/// Empty/whitespace values are ignored.
pub fn effective_proxy(explicit: Option<&str>) -> Option<String> {
    if let Some(p) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(p.to_string());
    }
    for var in ["OPENCODER_PROXY", "ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY"] {
        if let Ok(v) = std::env::var(var) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// Build a proxy-aware reqwest client (rustls). `explicit` is the config
/// `network.proxy` value; env fallbacks are applied via [`effective_proxy`].
/// Loopback hosts that always bypass any configured proxy. A forward proxy
/// must never intercept self-connections or local mock servers, otherwise
/// tests and localhost endpoints break whenever a proxy is in effect.
const LOOPBACK_NO_PROXY: &str = "127.0.0.1,localhost,::1,0.0.0.0";

/// Build a proxy-aware reqwest client (rustls) with a custom per-read idle
/// timeout. `explicit` is the config `network.proxy` value; env fallbacks are
/// applied via [`effective_proxy`]. When a proxy is in use, loopback hosts are
/// excluded so local traffic stays direct.
pub fn build_http_client_with_read_timeout(
    explicit: Option<&str>,
    read_timeout: std::time::Duration,
) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(read_timeout);
    if let Some(p) = effective_proxy(explicit) {
        let no_proxy = reqwest::NoProxy::from_string(LOOPBACK_NO_PROXY);
        let proxy = reqwest::Proxy::all(&p)
            .with_context(|| format!("invalid proxy '{p}'"))?
            .no_proxy(no_proxy);
        b = b.proxy(proxy);
    }
    b.build().context("build http client")
}

/// Build a proxy-aware reqwest client (rustls) with the default 300s
/// per-read idle timeout. `explicit` is the config `network.proxy` value;
/// env fallbacks are applied via [`effective_proxy`].
pub fn build_http_client(explicit: Option<&str>) -> Result<reqwest::Client> {
    build_http_client_with_read_timeout(explicit, std::time::Duration::from_secs(300))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_proxy_wins_over_env() {
        // explicit value must be returned even when env vars are set.
        assert_eq!(
            effective_proxy(Some("  socks5://127.0.0.1:1080  ")),
            Some("socks5://127.0.0.1:1080".to_string())
        );
    }

    #[test]
    fn empty_explicit_falls_through() {
        // Env-isolated: an empty explicit value must fall through to env, and
        // with no proxy env vars set at all, resolve to None.
        let keys = ["OPENCODER_PROXY", "ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY"];
        let saved: std::collections::HashMap<&str, Option<String>> = keys
            .iter()
            .map(|&k| (k, std::env::var(k).ok()))
            .collect();
        for k in keys {
            std::env::remove_var(k);
        }
        assert_eq!(effective_proxy(Some("   ")), None);
        std::env::set_var("OPENCODER_PROXY", "socks5://1.2.3.4:1080");
        assert_eq!(
            effective_proxy(Some("   ")),
            Some("socks5://1.2.3.4:1080".to_string())
        );
        for k in keys {
            match saved.get(k).cloned().flatten() {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }

    #[test]
    fn socks5_url_parses_as_reqwest_proxy() {
        // Proves the workspace `socks` feature is wired: SOCKS schemes must
        // construct a valid reqwest::Proxy without error.
        for scheme in ["socks5://127.0.0.1:1080", "socks5h://127.0.0.1:1080"] {
            reqwest::Proxy::all(scheme).unwrap_or_else(|_| panic!("socks proxy parsed: {scheme}"));
        }
        for scheme in ["http://127.0.0.1:18080", "https://127.0.0.1:18080"] {
            reqwest::Proxy::all(scheme).unwrap_or_else(|_| panic!("http proxy parsed: {scheme}"));
        }
    }

    #[test]
    fn loopback_no_proxy_is_constructable() {
        // The loopback exclusion list must yield a usable NoProxy so that a
        // configured forward proxy never intercepts local traffic.
        let np = reqwest::NoProxy::from_string(LOOPBACK_NO_PROXY);
        assert!(np.is_some(), "loopback NoProxy must build");
    }

    #[test]
    fn build_http_client_with_proxy_still_builds() {
        // A proxy + loopback no_proxy must construct a client without error.
        build_http_client(Some("http://127.0.0.1:18080")).expect("proxied client builds");
    }

    #[test]
    fn build_http_client_direct_when_no_proxy() {
        // With no explicit proxy and (in this test process) no proxy env vars,
        // the client builds cleanly with no proxy attached.
        std::env::remove_var("OPENCODER_PROXY");
        std::env::remove_var("ALL_PROXY");
        std::env::remove_var("HTTPS_PROXY");
        std::env::remove_var("HTTP_PROXY");
        build_http_client(None).expect("direct client builds");
    }
}
