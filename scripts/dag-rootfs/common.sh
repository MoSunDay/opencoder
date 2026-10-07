run_in_rootfs() {
  if [ "$(id -u)" -eq 0 ]; then
    chroot "$@"
  else
    sudo -n -- chroot "$@"
  fi
}

copy_libs() {
  local out="$1" source="$2" listing
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
  local out="$1" source="$2" guest="$3"
  mkdir -p "$out$(dirname "$guest")"
  cp -L "$source" "$out$guest"
  chmod 755 "$out$guest"
  copy_libs "$out" "$source"
}
