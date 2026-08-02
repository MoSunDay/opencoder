#!/usr/bin/env bash
#
# img2uri.sh — turn a local image into a data URI for the remote opencoder TUI.
#
# Workflow
# --------
# This script runs on YOUR LOCAL machine (where your screenshots live), NOT on
# the remote server. It reads an image (FILE argument, or the clipboard when no
# FILE is given), optionally compresses it with ImageMagick, base64-encodes it,
# and prints to stdout either:
#
#   1. default mode — a single line:
#
#        data:<mime>;base64,<payload>
#
#   2. --chunk mode — a frame protocol for huge images or terminals that
#      truncate very long pasted lines (e.g. tmux):
#
#        ocimg begin <id> <fmt> <total>
#        ocimg chunk <id> 0 <piece>
#        ocimg chunk <id> 1 <piece>
#        ...
#        ocimg end <id>
#
#      <id>    frame-set id, unique per file
#      <fmt>   canonical extension: png/jpeg/gif/webp/bmp
#      <total> number of chunks; chunk seq is 0-based
#      Each <piece> is at most KB*1024 base64 characters. The remote TUI
#      accumulates chunks by <id> until it sees `ocimg end`, then decodes.
#
# You copy the printed output and paste it into the remote opencoder TUI
# prompt, which decodes it back into an image attachment.

set -euo pipefail

CHUNK_MODE=0
CHUNK_KB=32
NO_COMPRESS=0
MAX_DIM=1600
QUALITY=82
IM_NOTE=0        # print the "ImageMagick not found" hint at most once per run
FILE_SEQ=0       # per-file counter, keeps chunk frame ids unique across files
FILES=()
TMP_FILES=()
NEW_TMP=""       # out-param of new_tmp (avoids a subshell so cleanup sees it)
CLIP_FILE=""     # out-param of read_clipboard

usage() {
  cat <<'EOF'
img2uri.sh [options] [FILE...]
  -h, --help      show this help
  --chunk [KB]    emit chunked `ocimg` frames (default chunk size 32 KB) instead of one data URI
  --no-compress   skip ImageMagick compression (default: compress when available)
  --max DIM       max dimension for compression (default 1600)
  --quality Q     JPEG quality for compression (default 82)
With no FILE, reads an image from the clipboard (wl-paste → xclip → pngpaste, auto-detected).
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  if [ "${#TMP_FILES[@]}" -gt 0 ]; then
    rm -f "${TMP_FILES[@]}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

# Create a temp file and register it for EXIT cleanup; path in $NEW_TMP.
# (Sets a global instead of echoing so the registration survives — command
# substitution would run this in a subshell and lose TMP_FILES.)
new_tmp() {
  NEW_TMP="$(mktemp "${TMPDIR:-/tmp}/img2uri.XXXXXX")"
  TMP_FILES+=("$NEW_TMP")
}

is_uint() {
  case "$1" in
    '' | *[!0-9]*) return 1 ;;
    *) return 0 ;;
  esac
}

# --- clipboard -----------------------------------------------------------

# Read an image from the clipboard into a temp file; result in $CLIP_FILE.
# Tries Wayland → X11 → macOS helper: the first tool that both EXISTS
# (command -v) and SUCCEEDS (non-empty output) wins.
read_clipboard() {
  local tmp found=0
  new_tmp
  tmp="$NEW_TMP"
  if command -v wl-paste >/dev/null 2>&1; then
    found=1
    if wl-paste -t image/png >"$tmp" 2>/dev/null && [ -s "$tmp" ]; then
      CLIP_FILE="$tmp"
      return 0
    fi
  fi
  if command -v xclip >/dev/null 2>&1; then
    found=1
    if xclip -selection clipboard -t image/png -o >"$tmp" 2>/dev/null && [ -s "$tmp" ]; then
      CLIP_FILE="$tmp"
      return 0
    fi
  fi
  if command -v pngpaste >/dev/null 2>&1; then
    found=1
    if pngpaste - >"$tmp" 2>/dev/null && [ -s "$tmp" ]; then
      CLIP_FILE="$tmp"
      return 0
    fi
  fi
  if [ "$found" -eq 0 ]; then
    {
      echo "error: no clipboard image tool found. Install one of:"
      echo "  wl-paste  (Wayland; package: wl-clipboard)"
      echo "  xclip     (X11; package: xclip)"
      echo "  pngpaste  (macOS; brew install pngpaste)"
    } >&2
  else
    {
      echo "error: could not read an image from the clipboard (tried: wl-paste, xclip, pngpaste)."
      echo "hint: copy a screenshot first, or pass a FILE argument."
    } >&2
  fi
  return 1
}

# --- mime / format ---------------------------------------------------------

# MIME by file extension (case-insensitive); fallback `file --mime-type`;
# ultimate fallback image/png.
mime_of() {
  local f="$1" ext m
  ext="${f##*.}"
  ext="$(printf '%s' "$ext" | tr '[:upper:]' '[:lower:]')"
  case "$ext" in
    png) printf 'image/png\n'; return 0 ;;
    jpg | jpeg) printf 'image/jpeg\n'; return 0 ;;
    gif) printf 'image/gif\n'; return 0 ;;
    webp) printf 'image/webp\n'; return 0 ;;
    bmp) printf 'image/bmp\n'; return 0 ;;
  esac
  if command -v file >/dev/null 2>&1; then
    m="$(file --mime-type -b "$f" 2>/dev/null || true)"
    case "$m" in
      image/png | image/jpeg | image/gif | image/webp | image/bmp)
        printf '%s\n' "$m"
        return 0
        ;;
    esac
  fi
  printf 'image/png\n'
}

# Canonical extension used as <fmt> in the chunk protocol.
fmt_of() {
  case "$1" in
    image/jpeg) printf 'jpeg\n' ;;
    image/gif) printf 'gif\n' ;;
    image/webp) printf 'webp\n' ;;
    image/bmp) printf 'bmp\n' ;;
    *) printf 'png\n' ;;
  esac
}

# --- encoding / compression ------------------------------------------------

# base64-encode a file onto ONE line; handles GNU (-w0) and BSD (tr) base64.
b64_of() {
  if base64 --help 2>&1 | grep -q -- '-w'; then
    base64 -w0 "$1"
  else
    base64 <"$1" | tr -d '\n'
  fi
}

# First ImageMagick binary on PATH (magick = IM7, convert = IM6), or empty.
magick_bin() {
  if command -v magick >/dev/null 2>&1; then
    printf 'magick\n'
  elif command -v convert >/dev/null 2>&1; then
    printf 'convert\n'
  fi
}

CUR_FILE="" # out-param of compress_image: bytes to encode
CUR_MIME="" # out-param of compress_image: their mime type

# Compress $1 (mime $2) down to a JPEG when possible. Fail-soft: on any
# ImageMagick error we warn on stderr and keep the ORIGINAL bytes. Never
# compresses gif/webp/bmp — those pass through untouched.
compress_image() {
  local in="$1" mime="$2"
  CUR_FILE="$in"
  CUR_MIME="$mime"
  if [ "$NO_COMPRESS" -eq 1 ]; then
    return 0
  fi
  case "$mime" in
    image/png | image/jpeg) ;;
    *) return 0 ;;
  esac
  local bin
  bin="$(magick_bin)"
  if [ -z "$bin" ]; then
    if [ "$IM_NOTE" -eq 0 ]; then
      IM_NOTE=1
      echo "note: ImageMagick not found; emitting uncompressed" >&2
    fi
    return 0
  fi
  local out
  new_tmp
  out="$NEW_TMP"
  rm -f "$out" # re-create with a .jpg suffix so ImageMagick writes JPEG
  out="$out.jpg"
  TMP_FILES+=("$out")
  # "${DIM}x${DIM}>" means "shrink only, never enlarge" — keep ">" quoted.
  if "$bin" "$in" -resize "${MAX_DIM}x${MAX_DIM}>" -strip -quality "$QUALITY" "$out" \
    2>/dev/null && [ -s "$out" ]; then
    CUR_FILE="$out"
    CUR_MIME="image/jpeg"
  else
    echo "warning: compression failed for '$in'; emitting uncompressed" >&2
    rm -f "$out"
  fi
  return 0
}

# --- output ----------------------------------------------------------------

# Default mode: exactly one data URI line.
emit_uri() {
  local mime="$1" file="$2"
  printf 'data:%s;base64,%s\n' "$mime" "$(b64_of "$file")"
}

# Chunk mode: fold the base64 payload into pieces of <= CHUNK_KB*1024 chars
# and print the `ocimg` frame protocol (seq 0-based, matching the TUI
# accumulator). The id carries a per-file counter so several files processed
# in the same second still get distinct frame sets.
emit_chunks() {
  local mime="$1" file="$2"
  local fmt id width total payload seq=0 piece
  fmt="$(fmt_of "$mime")"
  id="img-$(date +%s)-$$-${FILE_SEQ}"
  width=$((CHUNK_KB * 1024))
  payload="$(b64_of "$file")"
  # NOTE: feed fold with a trailing newline — GNU fold does not add one, and
  # both `wc -l` and the `read` loop below depend on the final line ending.
  total="$(printf '%s\n' "$payload" | fold -w "$width" | wc -l | tr -d ' ')"
  printf 'ocimg begin %s %s %s\n' "$id" "$fmt" "$total"
  while IFS= read -r piece; do
    printf 'ocimg chunk %s %s %s\n' "$id" "$seq" "$piece"
    seq=$((seq + 1))
  done < <(printf '%s\n' "$payload" | fold -w "$width")
  printf 'ocimg end %s\n' "$id"
}

# --- per-file pipeline -------------------------------------------------------

process_file() {
  local f="$1"
  if [ ! -f "$f" ]; then
    echo "error: no such file: $f" >&2
    return 1
  fi
  if [ ! -r "$f" ]; then
    echo "error: unreadable file: $f" >&2
    return 1
  fi
  local mime
  mime="$(mime_of "$f")"
  compress_image "$f" "$mime"
  if [ "$CHUNK_MODE" -eq 1 ]; then
    emit_chunks "$CUR_MIME" "$CUR_FILE"
  else
    emit_uri "$CUR_MIME" "$CUR_FILE"
  fi
}

# --- args / main -------------------------------------------------------------

parse_args() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -h | --help)
        usage
        exit 0
        ;;
      --chunk)
        CHUNK_MODE=1
        shift
        # KB is optional: consume the next arg only when it is numeric.
        if [ "$#" -gt 0 ] && is_uint "$1"; then
          if [ "$1" -eq 0 ]; then
            die "--chunk size must be > 0"
          fi
          CHUNK_KB="$1"
          shift
        fi
        ;;
      --no-compress)
        NO_COMPRESS=1
        shift
        ;;
      --max)
        [ "$#" -ge 2 ] || die "--max requires a value"
        is_uint "$2" || die "--max expects a positive integer, got '$2'"
        MAX_DIM="$2"
        shift 2
        ;;
      --quality)
        [ "$#" -ge 2 ] || die "--quality requires a value"
        is_uint "$2" || die "--quality expects an integer, got '$2'"
        QUALITY="$2"
        shift 2
        ;;
      --)
        shift
        while [ "$#" -gt 0 ]; do
          FILES+=("$1")
          shift
        done
        ;;
      -*)
        echo "error: unknown option: $1" >&2
        usage >&2
        exit 1
        ;;
      *)
        FILES+=("$1")
        shift
        ;;
    esac
  done
}

main() {
  parse_args "$@"
  if [ "${#FILES[@]}" -eq 0 ]; then
    if ! read_clipboard; then
      exit 1
    fi
    FILES=("$CLIP_FILE")
  fi
  # Process every file even if some fail; collect the exit code.
  local rc=0 f
  for f in "${FILES[@]}"; do
    if ! process_file "$f"; then
      rc=1
    fi
    FILE_SEQ=$((FILE_SEQ + 1))
  done
  exit "$rc"
}

main "$@"
