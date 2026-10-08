//! Proxy-aware HTTP client construction, shared by the LLM client and the
//! browser tools. Supports `socks5://` / `socks5h://` / `http://` / `https://`
//! proxies (the workspace `reqwest` enables the `socks` feature for SOCKS).

use anyhow::{Context, Result};

/// Resolve the effective proxy URL. Priority: an explicit config value, then
/// `OPENCODER_PROXY`, then `ALL_PROXY`, then `HTTPS_PROXY` / `HTTP_PROXY`.
/// Standard proxy variables also accept their lowercase spelling.
/// Empty/whitespace values are ignored.
pub fn effective_proxy(explicit: Option<&str>) -> Option<String> {
    proxy_url(
        explicit,
        [
            "OPENCODER_PROXY",
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ]
        .map(|key| std::env::var(key).ok()),
    )
}

fn proxy_url(
    explicit: Option<&str>,
    environment: impl IntoIterator<Item = Option<String>>,
) -> Option<String> {
    if let Some(p) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(p.to_string());
    }
    for value in environment.into_iter().flatten() {
        let t = value.trim();
        if !t.is_empty() {
            return Some(t.to_string());
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

/// Carry the existing proxy exclusions into otherwise empty OCI environments.
/// The proxy URL and credentials already live in the frozen configuration.
pub fn proxy_bypass_environment(lookup: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    ["NO_PROXY", "no_proxy"]
        .into_iter()
        .filter_map(|name| lookup(name).map(|value| (name.to_owned(), value)))
        .collect()
}

fn bypass_hosts(configured: Option<&str>) -> String {
    match configured.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => format!("{LOOPBACK_NO_PROXY},{value}"),
        None => LOOPBACK_NO_PROXY.to_owned(),
    }
}

/// Build a proxy-aware reqwest client (rustls) with a custom per-read idle
/// timeout. `explicit` is the config `network.proxy` value; env fallbacks are
/// applied via [`effective_proxy`]. When a proxy is in use, loopback hosts are
/// excluded so local traffic stays direct. `NO_PROXY` (or `no_proxy`) adds
/// the caller's excluded hosts without losing the loopback exclusions.
pub fn build_http_client_with_read_timeout(
    explicit: Option<&str>,
    read_timeout: std::time::Duration,
) -> Result<reqwest::Client> {
    let proxy = effective_proxy(explicit);
    let excluded = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .ok();
    build_http_client_with_proxy_rules(proxy.as_deref(), excluded.as_deref(), read_timeout)
}

/// Build a client from resolved proxy rules without reading process environment.
pub fn build_http_client_with_proxy_rules(
    proxy: Option<&str>,
    excluded: Option<&str>,
    read_timeout: std::time::Duration,
) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(read_timeout);
    if let Some(p) = proxy {
        let no_proxy = reqwest::NoProxy::from_string(&bypass_hosts(excluded));
        let proxy = reqwest::Proxy::all(p)
            .context("invalid proxy URL")?
            .no_proxy(no_proxy);
        b = b.proxy(proxy);
    }
    b.build().context("build http client")
}

/// Build a proxy-aware reqwest client (rustls) with the default 600s
/// per-read idle timeout. `explicit` is the config `network.proxy` value;
/// env fallbacks are applied via [`effective_proxy`].
pub fn build_http_client(explicit: Option<&str>) -> Result<reqwest::Client> {
    build_http_client_with_read_timeout(explicit, std::time::Duration::from_secs(600))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_inherits_proxy_exclusions_without_other_host_environment() {
        let environment = proxy_bypass_environment(|key| match key {
            "NO_PROXY" => Some("10.37.35.13,.internal".into()),
            "no_proxy" => Some("another.internal".into()),
            _ => panic!("unrelated host environment requested: {key}"),
        });
        assert_eq!(
            environment,
            vec![
                ("NO_PROXY".into(), "10.37.35.13,.internal".into()),
                ("no_proxy".into(), "another.internal".into()),
            ]
        );
        assert!(proxy_bypass_environment(|_| None).is_empty());
    }

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
        assert_eq!(proxy_url(Some("   "), [None]), None);
        assert_eq!(
            proxy_url(Some("   "), [Some("socks5://1.2.3.4:1080".into())]),
            Some("socks5://1.2.3.4:1080".to_string())
        );
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
        build_http_client_with_proxy_rules(None, None, std::time::Duration::from_secs(2))
            .expect("direct client builds");
    }
}
