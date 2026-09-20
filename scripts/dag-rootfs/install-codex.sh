#!/usr/bin/env bash
# Install a native Codex CLI and its shell/git runtime into a prepared rootfs.
# Auth/config are deliberately not copied: the executor mounts the node's
# original Codex home at launch, keeping OAuth refresh on one authoritative copy.
set -euo pipefail

out="${1:?rootfs directory required}"
codex_binary="${2:?native Codex binary required}"
[ -d "$out" ] || { echo "rootfs directory missing" >&2; exit 2; }
out="$(cd "$out" && pwd)"
[ -x "$codex_binary" ] || { echo "Codex binary is missing or not executable" >&2; exit 2; }
python3 - "$codex_binary" <<'PY'
import sys
with open(sys.argv[1], 'rb') as stream:
    if stream.read(4) != b'\x7fELF':
        sys.exit("Pass the native Codex ELF binary, not an npm/Python/shell launcher. Custom launchers and dependencies can instead be provisioned explicitly in the rootfs.")
PY

copy_libs() {
  local source="$1" listing
  # ldd returns nonzero for static ELF files, which need no shared libraries.
  listing="$(ldd "$source" 2>&1)" || {
    case "$listing" in
      *"not a dynamic executable"*|*"statically linked"*) return 0 ;;
      *) echo "$listing" >&2; return 1 ;;
    esac
  }
  case "$listing" in *"not found"*) echo "$listing" >&2; return 1 ;; esac
  while IFS= read -r lib; do
    [ -n "$lib" ] || continue
    mkdir -p "$out$(dirname "$lib")"
    cp -L "$lib" "$out$lib"
  done < <(printf '%s\n' "$listing" | awk '/=> \//{print $3} /^[[:space:]]*\//{print $1}')
}

install_binary() {
  local source="$1" guest="$2"
  mkdir -p "$out$(dirname "$guest")"
  cp -L "$source" "$out$guest"
  chmod 755 "$out$guest"
  copy_libs "$source"
}

install_binary "$codex_binary" /usr/bin/codex
install_binary /bin/sh /bin/sh
install_binary /bin/bash /bin/bash
for tool in env cat ls pwd mkdir cp mv rm head tail sed grep find sort wc sleep git; do
  source="$(type -P "$tool")" || { echo "required tool missing: $tool" >&2; exit 2; }
  install_binary "$source" "/usr/bin/$tool"
done
# Older glibc loads NSS modules at runtime; ldd does not list them. Keep
# both hosts-file and DNS resolution working for model endpoints and proxies.
libc="$(ldd /bin/sh | awk '$1 == "libc.so.6" {print $3; exit}')"
if [ -n "$libc" ]; then
  for name in libnss_files.so.2 libnss_dns.so.2; do
    lib="$(dirname "$libc")/$name"
    [ ! -f "$lib" ] || install_binary "$lib" "$lib"
  done
fi
cp -L /etc/hosts "$out/etc/hosts"
# Git HTTP transports live outside the git binary and need their own libraries.
git_exec="$(git --exec-path)"
mkdir -p "$out$git_exec"
cp -a "$git_exec/." "$out$git_exec/"
for helper in git-remote-http git-remote-https; do
  [ ! -x "$git_exec/$helper" ] || copy_libs "$git_exec/$helper"
done
mkdir -p "$out/etc/ssl/certs"
cp -L /etc/ssl/certs/ca-certificates.crt "$out/etc/ssl/certs/ca-certificates.crt"
# Codex runs shell commands with C.UTF-8. Older glibc releases store that
# locale on disk; include it so Unicode tools do not start with locale errors.
for locale_dir in /usr/lib/locale/C.UTF-8 /usr/lib/locale/C.utf8; do
  if [ -d "$locale_dir" ]; then
    mkdir -p "$out/usr/lib/locale"
    cp -aL "$locale_dir" "$out/usr/lib/locale/"
  fi
done
printf 'root:x:0:0:root:/tmp/codex-user:/bin/bash\n' > "$out/etc/passwd"
printf 'root:x:0:\n' > "$out/etc/group"
printf 'hosts: files dns\n' > "$out/etc/nsswitch.conf"
echo "==> Codex installed at /usr/bin/codex with shell, Git, TLS and shared libraries"
