#!/usr/bin/env bash
set -euo pipefail
out="${1:?rootfs directory required}"
[ -d "$out" ] || { echo "rootfs directory missing" >&2; exit 2; }
out="$(cd "$out" && pwd)"
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
install_binary "$out" /bin/sh /bin/sh
install_binary "$out" /bin/bash /bin/bash
for tool in env cat ls pwd mkdir cp mv rm head tail sed grep find sort wc sleep git; do
  binary="$(type -P "$tool")" || { echo "required tool missing: $tool" >&2; exit 2; }
  install_binary "$out" "$binary" "/usr/bin/$tool"
done
libc="$(ldd /bin/sh | awk '$1 == "libc.so.6" {print $3; exit}')"
if [ -n "$libc" ]; then
  for name in libnss_files.so.2 libnss_dns.so.2; do
    library="$(dirname "$libc")/$name"
    [ ! -f "$library" ] || install_binary "$out" "$library" "$library"
  done
fi
mkdir -p "$out/etc"
cp -L /etc/hosts "$out/etc/hosts"
git_exec="$(git --exec-path)"
mkdir -p "$out$git_exec"
cp -aL "$git_exec/." "$out$git_exec/"
for helper in git-remote-http git-remote-https; do
  [ ! -x "$git_exec/$helper" ] || copy_libs "$out" "$git_exec/$helper"
done
mkdir -p "$out/etc/ssl/certs"
cp -L /etc/ssl/certs/ca-certificates.crt "$out/etc/ssl/certs/ca-certificates.crt"
for locale_dir in /usr/lib/locale/C.UTF-8 /usr/lib/locale/C.utf8; do
  if [ -d "$locale_dir" ]; then
    mkdir -p "$out/usr/lib/locale"
    cp -aL "$locale_dir" "$out/usr/lib/locale/"
  fi
done
printf 'root:x:0:0:root:/tmp:/bin/bash\n' > "$out/etc/passwd"
printf 'root:x:0:\n' > "$out/etc/group"
printf 'hosts: files dns\n' > "$out/etc/nsswitch.conf"
run_in_rootfs "$out" /bin/bash -lc 'printf "native shell ready\n"'
