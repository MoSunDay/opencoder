---
name: chrome-headless
description: Headless Chrome rendering via CLI. Fetch JavaScript-heavy pages, unwind search-engine redirect links, dump raw DOM for structured extraction, and take screenshots using a real Chrome/Chromium binary. Upgrade path beyond web_fetch for pages that need full JS rendering or visual capture. Requires Chrome or Chromium installed.
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
   Search-engine result pages (Baidu/Bing/Sogou/DDG) are auto-detected and
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

## Structured extraction: `web_extract`

`web_extract` distills raw HTML into `{title, author, date, url, content}`.
Built-in site profiles: **知乎** (zhihu.com / zhuanlan.zhihu.com), **CSDN**,
**掘金**, **公众号** (mp.weixin.qq.com, incl. author + date), **博客园**,
**Wikipedia** (zh/en), **GitHub** (og:title), **36kr**, **StackOverflow**.
Other sites fall back to generic readable-text extraction.

## Deepresearch workflow

For a research question, prefer the single-call `research` tool:
`research(query="...", max_results=6, engines=["bing", "baidu"])` — it
renders each engine's SERP, merges + dedups results, renders every result,
extracts articles, and writes a markdown report to `.research/<slug>-<ts>.md`.

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
