#!/bin/sh
# Starts dnsui. When the container starts as root, this script makes the mounted
# config directory writable by the dnsui user, then drops to that user.
#
# The Config page saves by writing a temp file in the config directory and renaming
# it over config.yaml. That needs the directory to be writable by dnsui. On a Linux
# host the bind-mounted directory is often owned by root, which caused
# "writing config.yaml.tmp: Permission denied".
set -e

CONFIG_DIR="${DNSUI_CONFIG_DIR:-/app/config}"

if [ "$(id -u)" = "0" ]; then
  if [ -d "$CONFIG_DIR" ] && ! su-exec dnsui:dnsui sh -c 'test -w "$1"' _ "$CONFIG_DIR"; then
    chown dnsui:dnsui "$CONFIG_DIR" 2>/dev/null \
      || echo "warning: cannot make $CONFIG_DIR writable for dnsui; saving the config will fail" >&2
    if [ -f "$CONFIG_DIR/config.yaml" ]; then
      chown dnsui:dnsui "$CONFIG_DIR/config.yaml" 2>/dev/null || true
    fi
  fi
  exec su-exec dnsui:dnsui dnsui "$@"
fi

exec dnsui "$@"
