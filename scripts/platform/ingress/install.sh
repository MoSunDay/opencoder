#!/usr/bin/env bash
# One-time ingress installation, separate from compatible business releases.
# Requires a C compiler, make, curl, PCRE, zlib and OpenSSL development headers.
set -euo pipefail
version=1.30.4
expected=4261dc90e9e47c1c4041276e9aaa3d48ebe2e664f728e14fa95ae6c67d57a08b
if command -v nginx >/dev/null 2>&1; then
  nginx -v
  exit 0
fi
if [[ -e /etc/nginx/nginx.conf || -e /etc/systemd/system/nginx.service ]]; then
  echo 'Existing ingress configuration requires an explicit maintenance review.' >&2
  exit 1
fi
build_dir="$(mktemp -d /tmp/opencoder-ingress.XXXXXX)"
curl --fail --location --silent --show-error "https://nginx.org/download/nginx-$version.tar.gz" -o "$build_dir/source.tar.gz"
actual="$(sha256sum "$build_dir/source.tar.gz")"
[[ "${actual%% *}" == "$expected" ]] || { echo 'Nginx source checksum mismatch' >&2; exit 1; }
tar -xzf "$build_dir/source.tar.gz" -C "$build_dir"
cd "$build_dir/nginx-$version"
./configure --prefix=/usr/local/lib/opencoder-nginx --sbin-path=/usr/local/sbin/nginx \
  --conf-path=/etc/nginx/nginx.conf --pid-path=/run/opencoder-nginx.pid \
  --error-log-path=/var/log/opencoder-nginx/error.log --http-log-path=/var/log/opencoder-nginx/access.log \
  --with-pcre-jit --with-http_ssl_module
make -j4
install -d /etc/nginx/conf.d /var/log/opencoder-nginx /usr/local/lib/opencoder-nginx /usr/local/sbin
install -m 755 objs/nginx /usr/local/sbin/nginx
install -m 644 conf/mime.types /etc/nginx/mime.types
cat > /etc/nginx/nginx.conf <<'CONF'
worker_processes auto;
worker_rlimit_nofile 65536;
pid /run/opencoder-nginx.pid;
error_log /var/log/opencoder-nginx/error.log warn;
events { worker_connections 4096; }
http {
    include /etc/nginx/mime.types;
    default_type application/octet-stream;
    access_log /var/log/opencoder-nginx/access.log;
    sendfile on;
    keepalive_timeout 65;
    include /etc/nginx/conf.d/*.conf;
}
CONF
cat > /etc/systemd/system/nginx.service <<'UNIT'
[Unit]
Description=OpenCoder stable Nginx ingress
After=network.target
[Service]
Type=simple
LimitNOFILE=1048576
ExecStartPre=/usr/local/sbin/nginx -t
ExecStart=/usr/local/sbin/nginx -g "daemon off;"
ExecReload=/usr/local/sbin/nginx -s reload
KillSignal=SIGQUIT
KillMode=process
TimeoutStopSec=infinity
SendSIGKILL=no
Restart=on-failure
[Install]
WantedBy=multi-user.target
UNIT
nginx -t
systemctl daemon-reload
printf 'Ingress installed. First migration will start its configured listeners.\n'
