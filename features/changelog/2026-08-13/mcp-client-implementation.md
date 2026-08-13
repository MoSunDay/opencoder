# MCP Client Implementation

**Date:** 2026-08-13
**Status:** Implemented
**Scope:** `crates/session/src/mcp/` (new module), `crates/session/src/runner/`, `crates/session/src/prompt.rs`, `crates/session/src/compaction.rs`, `crates/web/`, `crates/tui/`

## Summary

Implemented full MCP (Model Context Protocol) client support. Sessions with at least one `enabled == true` MCP server now connect to those servers via stdio, discover their tools, and register them in the tool table so the LLM can call them via function-calling like any builtin tool. Sessions without enabled servers have zero MCP overhead.

## Changes

### New Module: `crates/session/src/mcp/` (6 files, ~990 lines)

| File | Lines | Responsibility |
|---|---|---|
| `mod.rs` | 19 | Module declarations + re-exports |
| `protocol.rs` | 222 | JSON-RPC 2.0 + MCP protocol types (serde, camelCase aliases) |
| `transport.rs` | 221 | `McpTransport` trait, `StdioTransport` (child process), `MockTransport` (tests) |
| `client.rs` | 282 | `McpClient` — async JSON-RPC client with request/response routing, `initialize`/`list_tools`/`call_tool` |
| `tool.rs` | 231 | `McpTool` implementing `Tool` trait; `build_tools()` factory; `mcp__` prefix helpers |
| `pool.rs` | 232 | Process-global `MCP_POOL` keyed by session_id; `sync()`/`tools_for()`/`status_for()`/`cleanup()` |

### Modified Files

| File | Change |
|---|---|
| `session/src/lib.rs` | Added `pub mod mcp;` |
| `session/src/runner/mod.rs` | `run()`/`run_with_images()` now call `build_full_registry()` which syncs MCP pool + merges MCP tools |
| `session/src/runner/llm_call.rs` | ToolFilter: `mcp__` tools visible to all non-Subagent agents; `mcp_section()` input from pool status |
| `session/src/prompt.rs` | `mcp_section()` signature changed to `&[(String, ConnStatus)]` — lists connected servers + tool counts |
| `session/src/compaction.rs` | `mcp_section()` call updated to use pool status |
| `web/src/handle.rs` | `release_events_subscriber()`: MCP cleanup on eviction; `ReloadConfig`: MCP pool sync |
| `web/src/api.rs` | `DELETE /sessions/:id`: MCP cleanup before delete |
| `tui/src/worker.rs` | `ReloadConfig`: MCP pool sync after config reload |
| `session/Cargo.toml` | Added `[[bin]] mcp_mock_server` for integration tests |

### New Test Files

| File | Tests |
|---|---|
| `session/bin/mcp_mock_server.rs` | Mock MCP server binary (echo + add tools) |
| `session/tests/mcp_integration.rs` | 7 integration tests |

## Design Decisions

1. **No external MCP crate (rmcp)** — hand-written JSON-RPC 2.0 for minimal dependencies and lifecycle control.
2. **Process-global pool** (`static MCP_POOL`) — follows the session_id-keyed pattern; connections survive per-drain SessionState rebuilds in web mode.
3. **`mcp__{server}__{tool}` naming** — avoids builtin collisions; ToolFilter identifies MCP tools by prefix.
4. **Subagent exclusion** — explore/build subagents never see MCP tools (sandboxed).
5. **Graceful failure** — a failed server is recorded as `ConnStatus::Failed`, surfaced in system prompt; other servers are unaffected.
6. **stdio transport P0** — covers npx/node/python MCP servers; SSE is P1 (returns error).

## Test Coverage

### Unit Tests (20, inline in mcp/ modules)

| Module | Tests | What they verify |
|---|---|---|
| `protocol.rs` | 5 | JSON-RPC roundtrip, camelCase aliases, ToolCallResult parsing |
| `transport.rs` | 2 | MockTransport bidirectional roundtrip |
| `client.rs` | 4 | initialize handshake, list_tools, call_tool, error propagation |
| `tool.rs` | 4 | execute ok/error, prefix check, full name assignment |
| `pool.rs` | 5 | empty session, cleanup, noop sync, bad command failure |

### LLM Call Filter Tests (2, in `llm_call.rs`)

| Test | Verifies |
|---|---|
| `mcp_tools_visible_to_act_agent` | Act agent's schema includes `mcp__` tools |
| `mcp_tools_hidden_from_subagent` | Subagent's schema excludes all `mcp__` tools |

### Integration Tests (7, `tests/mcp_integration.rs`)

| Test | Verifies |
|---|---|
| `sync_connects_and_discovers_tools` | Real stdio connection → 2 tools discovered |
| `call_echo_tool_via_registry` | echo tool returns correct text |
| `call_add_tool_returns_sum` | add tool computes 7+35=42 |
| `sync_idempotent_does_not_reconnect` | Same desired list = no reconnect |
| `disable_server_removes_tools` | Empty desired = tools removed |
| `bad_command_records_failed_status` | Nonexistent binary = Failed status, no panic |
| `cleanup_disconnects_and_clears_pool` | cleanup() empties pool |

### Prompt Tests (4, in `prompt.rs`)

| Test | Verifies |
|---|---|
| `mcp_section_empty_returns_none` | No connections = None |
| `mcp_section_connected_shows_tool_count` | Connected server shows tool count |
| `mcp_section_failed_shows_error_message` | Failed server shows error |
| `mcp_section_mixed_statuses` | Multiple servers with mixed status |

**Total new tests: 33** (20 unit + 2 filter + 7 integration + 4 prompt)

## Verification

```
cargo test --workspace           → 2455 passed, 0 failed
cargo clippy --workspace -D warnings → 0 warnings
cargo build --workspace           → clean
```
