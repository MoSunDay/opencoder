# ctx-text-restores-model-window

## Summary

The status bar's trailing `ctx N%` text and progress bar now track
**different** denominators, restoring an accurate model-window readout.

- The progress bar `▰▰▱▱` **and its colour** are driven by
  `compaction_threshold` (100 % = compaction trigger point), unchanged.
- The trailing text `ctx N % (used / limit)` is now driven by
  `context_limit` — the **model context window** — so it no longer mirrors
  the compaction bar and reflects real context exhaustion against the model
  ceiling.

Previously both the bar and the text used `compaction_threshold`, so the
textual percentage was a compaction-progress indicator rather than a true
context-window usage figure. `CONTEXT_BASELINE = 4000` tokens is subtracted
from both metrics so small sessions display ~0 %.

## Changes

### `crates/tui/src/render.rs`
- `render_status` now receives both `compaction_threshold` and `context_limit`.
- Bar fill `bar_pct` and threshold colour continue to use `compaction_threshold`.
- Trailing `win_pct` / `(used / limit)` text now uses `context_limit`.

### `crates/tui/src/app.rs`
- `run_app` threads `context_limit` (read from the reloaded `Config`) into
  `render_status` and into the `handle_model_outcome` call site.

### `crates/tui/src/app_loop_model.rs` (NEW, ~145 lines)
- Extracts `handle_model_outcome` out of `app_loop.rs` so the latter stays
  under the 800-line iteration cap. Pure relocation — behaviour unchanged.
- Registered via `#[path = "app_loop_model.rs"] mod app_loop_model;` +
  `pub(crate) use`.

### `crates/tui/src/app_loop.rs`
- Loses the inlined `handle_model_outcome` (−~127 lines) and the now-unused
  imports (`Duration`, `ChatStream`, `handle_model_key`, `ModelOutcome`,
  `switch_session`); gains the `mod app_loop_model` / `use` pair.

### Tests
- `render_tests/status_bar.rs` / `status_ctx.rs` / `timer.rs` and
  `render_clear_tests.rs` updated to assert the split bar/text semantics.
