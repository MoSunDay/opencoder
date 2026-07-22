---
name: chrome-headless
description: Headless Chrome rendering via CLI. Fetch JavaScript-heavy pages and take screenshots using a real Chrome/Chromium binary. Upgrade path beyond web_fetch for pages that need full JS rendering or visual capture. Requires Chrome or Chromium installed.
---

# chrome-headless skill

You have a tool that drives a real Chrome/Chromium binary in headless mode.
Use it when `web_fetch` is unavailable or insufficient (JS-heavy SPAs, pages
that need screenshot capture).

## Actions

1. **fetch** — render a URL and extract readable text:
   `chrome_headless(action="fetch", url="https://example.com")`
   Optional: `wait` (milliseconds to wait for JS), `selector` (CSS selector
   to focus extraction).

2. **screenshot** — capture a full-page screenshot to a file:
   `chrome_headless(action="screenshot", url="https://example.com")`
   Returns the file path. Use the `read` tool to inspect the image.

## When to use

- **Prefer `web_fetch` first** if available — it is faster and lighter.
- Use `chrome_headless` when:
  - `web_fetch` returns empty/broken content (heavy client-side rendering).
  - You need a visual screenshot for debugging UI issues.
  - The page blocks automated fetchers but renders in a real browser.

## Notes

- Each call spawns a short-lived Chrome process (no persistent session).
- `--no-sandbox` is used automatically (required in containers/CI).
- Chrome binary is auto-detected: `google-chrome`, `google-chrome-stable`,
  `chromium-browser`, `chromium`, or `$CHROME_PATH`.
