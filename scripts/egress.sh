#!/bin/sh
# HD-11 egress: the executable form of "local only". Runs a command under strace, following every
# process it starts, and fails if any of them calls connect() on an IPv4 or IPv6 address that isn't
# loopback, whether or not the connection succeeds. Unix sockets and netlink are allowed.
#
#   scripts/egress.sh cargo nextest run -p cartograph-cli --offline
#
# Linux only (CI runs it on ubuntu-latest). The deliberate violation that proves it works:
#   scripts/egress.sh python3 -c 'import socket; socket.socket().connect_ex(("198.51.100.1", 9))'
# must exit 1 and name 198.51.100.1.
#
# ponytail: strace on connect() instead of a network namespace: it catches the attempt itself, and
# needs no root or user namespaces (which Ubuntu 24.04's AppArmor restricts).
set -u
log=$(mktemp)
trap 'rm -f "$log"' EXIT
strace -f -qq --seccomp-bpf -e trace=connect -e signal=none -o "$log" "$@"
status=$?
# A connect line names its family and address, e.g.
#   sin_addr=inet_addr("1.2.3.4")  or  inet_pton(AF_INET6, "2001:db8::1", &sin6_addr)
bad=$(grep -E 'sa_family=AF_INET6?,' "$log" |
  grep -vE 'inet_addr\("127\.|inet_pton\(AF_INET6, "(::1|::ffff:127\.[0-9.]+)"' || true)
if [ -n "$bad" ]; then
  echo "::error::outbound connection attempted (HD-11 egress):"
  printf '%s\n' "$bad"
  exit 1
fi
echo "egress: $(grep -c 'connect(' "$log") connect() calls, all loopback or local sockets"
exit "$status"
