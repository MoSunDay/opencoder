---
name: chrome-headless
description: Site-scoped web search and DOM extraction by driving a locally installed headless Chrome/Chromium through the bash tool, with optional HTTP proxy tunneling. No browser binary is bundled - the agent locates Chrome at runtime. API-first for GitHub and HuggingFace (curl against their public JSON APIs); headless DOM rendering for Google SERP and JS-heavy pages. Requires a local Chrome or Chromium.
---

# chrome-headless skill

You can reach the web by driving a locally installed headless browser through the
**bash** tool. There is no dedicated browser tool - every action below is an
ordinary shell command. Prefer structured public APIs (GitHub, HuggingFace); fall
back to headless DOM rendering only when no API exists or the page is JS-rendered.

## Prerequisites

- A local Chrome or Chromium binary. Locate it first (see *Detect Chrome*).
- If absent, tell the user how to install it; do not run a package manager
  unattended unless explicitly asked.
- The bash tool truncates large output. For anything beyond a quick check,
  redirect DOM/API output to a temp file and inspect it with the read tool.

## Detect Chrome

    CHROME="$(command -v google-chrome google-chrome-stable chromium chromium-browser chrome 2>/dev/null | head -1)"

If CHROME is empty, Chrome is not installed - stop and ask the user.

## Core primitives

Dump rendered DOM to a file (handles JS-rendered pages):

    "$CHROME" --headless=new --disable-gpu --no-sandbox \
      --user-agent="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36" \
      --dump-dom "https://example.com/page" > /tmp/dom.html 2>/tmp/chrome.err

Then read /tmp/dom.html (or grep it) and check /tmp/chrome.err on failure.

HTTP proxy tunneling (append to any --dump-dom invocation):

      --proxy-server="http://HOST:PORT" \
      --proxy-bypass-list="<local>;127.0.0.1;localhost;::1;0.0.0.0"

The bypass list is semicolon-separated. Use it only when the user provides a proxy.

Plain HTTP via curl (no rendering needed - faster, no browser required):

    curl -sSL -A "Mozilla/5.0" "https://example.com/api/endpoint" > /tmp/out.json

## Site search matrix

### GitHub - API first
Code, repos, and issues via the Search API:

    curl -sSL -G "https://api.github.com/search/code" \
      --data-urlencode "q=KEYWORDS" -H "Accept: application/vnd.github+json" \
      ${GITHUB_TOKEN:+-H "Authorization: Bearer $GITHUB_TOKEN"} > /tmp/gh.json

Use /search/repositories and /search/issues analogously. Rate limits:
unauthenticated search is about 10 req/min; general about 60 req/hr. If
GITHUB_TOKEN is set in the environment, always send it. Reserve DOM scraping for
content the API does not expose (e.g. rendered README sections).

### HuggingFace - API first
Models, datasets, spaces:

    curl -sSL -G "https://huggingface.co/api/models" \
      --data-urlencode "search=KEYWORDS" > /tmp/hf.json

Use /api/datasets and /api/spaces analogously. Fetch a model card (metadata plus
description) via https://huggingface.co/api/models/OWNER/NAME . Fall back to
--dump-dom on huggingface.co/OWNER/NAME only to capture rendered widget output.

### Google - DOM fallback (no official free API)
Site-scoped SERP via headless rendering:

    "$CHROME" --headless=new --disable-gpu --no-sandbox \
      --user-agent="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36" \
      --dump-dom "https://www.google.com/search?q=site:github.com+KEYWORDS" \
      > /tmp/serp.html

Then extract result links/titles from /tmp/serp.html (results sit in div.g; titles
in h3; URLs may be Google redirect wrappers around url?q= ). Google actively
blocks automation - expect CAPTCHAs or empty pages. If rendering fails, prefer
the target site's own search or its API (GitHub/HuggingFace above).

## Rules

- **API first.** Never scrape a page that exposes a JSON API. GitHub and
  HuggingFace must go through their APIs.
- **Be polite.** Avoid tight request loops; reuse cached output files within a
  task. Respect rate limits and Retry-After.
- **Never log secrets.** Send tokens only as headers; never echo GITHUB_TOKEN or
  proxy credentials into output you return or persist.
- **Output discipline.** Dump large responses to /tmp and read selectively; do
  not paste megabyte HTML into the conversation.
- **Proxy is opt-in.** Add --proxy-server / --proxy-bypass-list only when the
  user supplies a proxy endpoint.
- **No unattended installs.** If Chrome is missing, ask the user rather than
  running a package manager unprompted.
