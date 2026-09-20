#!/bin/sh
set -eu
prompt="$(cat)"
printf '%s\n' "$@" > "$OPENCODER_STEP_DIR/argv.txt"
[ "$(cat "$CODEX_HOME/auth.json")" = fixture-login ] || { echo "fixture authentication rejected" >&2; exit 1; }
# Mount namespace isolation and writable step/home are real kernel properties.
[ -d /proc/1 ] && [ -d "$HOME" ]
if (printf forbidden > /usr/bin/codex-write-probe) 2>/dev/null; then exit 31; fi
if (printf forbidden > "$OPENCODER_KNOWLEDGE_DIR/write-probe") 2>/dev/null; then exit 32; fi
cat "$CODEX_HOME/auth.json" > "$CODEX_HOME/refresh-$OPENCODER_STEP_SESSION_ID"
mv "$CODEX_HOME/refresh-$OPENCODER_STEP_SESSION_ID" "$CODEX_HOME/auth.json"
printf refreshed > "$CODEX_HOME/refresh-observed"
printf '%s\n' '{"type":"thread.started","thread_id":"runc-codex-thread"}' '{"type":"turn.started"}'
printf started > "$OPENCODER_STEP_DIR/started"
case "$prompt" in
  *CODEX_AUTH_FAIL*) echo "fixture authentication rejected" >&2; exit 1 ;;
  *CODEX_MALFORMED*) echo 'invalid JSON stream'; exit 1 ;;
  *CODEX_WAIT*) sleep 120 ;;
  *CODEX_SECOND*) case "$prompt" in *'"runc": true'*) : ;; *) exit 33 ;; esac ;;
esac
printf '%s\n' '{"type":"item.completed","item":{"type":"command_execution","id":"tool","command":"check container mounts","aggregated_output":"runc-tool-ok","status":"completed","exit_code":0}}'
printf '%s\n' '{"type":"item.completed","item":{"type":"agent_message","id":"answer","text":"```json\n{\"runc\":true}\n```"}}'
printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":8,"output_tokens":4}}'
