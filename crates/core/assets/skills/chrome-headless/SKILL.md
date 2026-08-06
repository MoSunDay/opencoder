---
name: chrome-headless
description: Headless Chrome rendering via CLI. Fetch JavaScript-heavy pages, unwind search-engine redirect links, dump raw DOM for structured extraction, and take screenshots using a real Chrome/Chromium binary. Supports HTTP/SOCKS5 proxy via config and env vars, and multi-engine SERP detection (Bing, Baidu, Sogou, DDG, Google, GitHub, HuggingFace). Upgrade path beyond web_fetch for pages that need full JS rendering or visual capture. Requires Chrome or Chromium installed.
---

# chrome-headless skill

You have a tool that drives a real Chrome/Chromium binary in headless mode.
Use it when `web_fetch` is unavailable or insufficient (JS-heavy SPAs, pages
that need screenshot capture). It is also the workhorse of the deepresearch
workflow (see below): search-engine SERPs are rendered with a real Chrome UA,
and result pages are unwound through their redirect links and distilled into
structured articles with `web_extract`.

## Actions

1. **fetch** — render a URL and extract readable text:
   `chrome_headless(action="fetch", url="https://example.com")`
   Search-engine result pages (Baidu/Bing/Sogou/DDG/Google) and site-search
   pages (GitHub repositories, HuggingFace models) are auto-detected and
   returned as a numbered markdown list of `{title, snippet, url}` rows.
   Optional: `wait` (ms of virtual time for JS), `ua` (override the UA).

2. **resolve** — unwind a search-engine redirect link to the real target:
   `chrome_headless(action="resolve", url="https://www.baidu.com/link?url=...")`
   Chrome follows the redirect, then the real URL is read from the rendered
   page's `<link rel="canonical">` / `og:url`. Returns the resolved URL, page
   title and a short excerpt. **This is what makes Baidu/Sogou SERP links
   actually usable** — their redirect URLs are not decodable client-side.

3. **html** — dump the raw rendered DOM (truncated):
   `chrome_headless(action="html", url="https://example.com")`
   Feed the result to `web_extract(html=..., url=...)` for site-aware
   article extraction.

4. **screenshot** — capture a full-page screenshot to a file:
   `chrome_headless(action="screenshot", url="https://example.com")`
   Returns the file path. Use the `read` tool to inspect the image.

## Proxy configuration

Chrome automatically honors `config.network.proxy` and the standard proxy env
vars, checked in order: `OPENCODER_PROXY`, `ALL_PROXY`, `HTTPS_PROXY`,
`HTTP_PROXY`. The chosen proxy URL is passed to Chrome as
`--proxy-server=<url>`. Supported schemes: `http://`, `https://`,
`socks5://`, `socks5h://` (use the `h` variants to resolve DNS through the
proxy). Loopback hosts (`localhost`, `127.0.0.1`, `::1`, etc.) always bypass
the proxy via `--proxy-bypass-list`, so local services keep working.

## Structured extraction: `web_extract`

`web_extract` distills raw HTML into `{title, author, date, url, content}`.
Built-in site profiles: **知乎** (zhihu.com / zhuanlan.zhihu.com), **CSDN**,
**掘金**, **公众号** (mp.weixin.qq.com, incl. author + date), **博客园**,
**Wikipedia** (zh/en), **GitHub** (og:title), **HuggingFace** (model cards),
**36kr**, **StackOverflow**.
Other sites fall back to generic readable-text extraction.

## Site search matrix

| engine | URL template | Notes |
|--------|-------------|-------|
| `bing` | `cn.bing.com/search?q=` | Default engine |
| `baidu` | `baidu.com/s?wd=` | Chinese results, redirect links need `resolve` |
| `sogou` | `sogou.com/web?query=` | Alternative, redirect links need `resolve` |
| `ddg` | `html.duckduckgo.com/html/?q=` | Privacy-focused |
| `google` | `google.com/search?q=&hl=en&num=10` | CAPTCHA risk — fall back to bing/ddg |
| `github` | `github.com/search?q=&type=repositories` | Repo search, SSR results |
| `hf` | `huggingface.co/models?search=` | Model search |

## Site search tips

- Google may serve a CAPTCHA to headless Chrome; if the SERP looks empty or
  asks you to prove you are human, fall back to Bing/DDG.
- GitHub and HuggingFace return results server-side rendered, so their links
  are already final URLs — no `resolve` step needed.

## Deepresearch workflow

For a research question, prefer the single-call `research` tool:
`research(query="...", max_results=6, engines=["bing", "baidu"])` — it
renders each engine's SERP, merges + dedups results, renders every result,
extracts articles, and writes a markdown report to `.research/<slug>-<ts>.md`.

For technical research, mix a broad web engine with source registries:
`research(query="...", engines=["google", "github", "hf"])`. Google renders
the broad SERP while GitHub/HuggingFace return repository and model matches
directly. Google may trigger CAPTCHA under headless — if it does, fall back
to Bing/DDG for the web leg.

Doing it manually (when you need fine-grained control):

1. `chrome_headless(action="fetch", url=<serp>)` per engine — pick the SERP
   URLs from `research`'s engine list (Bing `cn.bing.com/search?q=`, Baidu
   `baidu.com/s?wd=`, Sogou `sogou.com/web?query=`, DDG
   `html.duckduckgo.com/html/?q=`).
2. For Baidu/Sogou results (redirect URLs), call
   `chrome_headless(action="resolve", url=<redirect>)` to get the real URL.
3. `chrome_headless(action="html", url=<real-url>)` then
   `web_extract(html=<dom>, url=<real-url>)` to get a clean article.
4. Collect 3-6 sources, then summarize (or use `research` to do it all).

Engine failures: Baidu/Sogou may serve an anti-bot wall; skip that engine and
fall back to Bing/DDG. No single engine should block the pipeline.

## Notes

- Each call spawns a short-lived Chrome process (no persistent session).
- A real Chrome UA + `AutomationControlled`-off are used by default; override
  per call with the `ua` input if a site requires it.
- `--no-sandbox` and a fresh per-call profile dir are used automatically
  (required in containers/CI; also avoids profile-in-use races).
- Chrome binary is auto-detected: `google-chrome`, `google-chrome-stable`,
  `chromium-browser`, `chromium`, or `$CHROME_PATH`.
