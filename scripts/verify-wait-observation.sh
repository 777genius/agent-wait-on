#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test -f tests/wait_observation.rs || { printf '%s
' 'BLOCK: missing tests/wait_observation.rs' >&2; exit 1; }
listing=$(cargo test --test wait_observation -- --list)
for name in stale_running_cli stale_running_reference stale_running_mcp stale_running_http live_quiet_job finite_budget delayed_terminal concurrent_observers; do
  case "$listing" in
    *"$name: test"*) ;;
    *) printf 'BLOCK: missing collected test %s
' "$name" >&2; exit 1 ;;
  esac
done
cargo test --test wait_observation
