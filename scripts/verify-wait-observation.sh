#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test -f tests/wait_observation.rs || { printf '%s\n' 'BLOCK: missing tests/wait_observation.rs' >&2; exit 1; }
require_test() {
  case "
$listing
" in
    *"
$1: test
"*) ;;
    *) printf 'BLOCK: missing collected test %s\n' "$1" >&2; exit 1 ;;
  esac
}
listing=$(cargo test --test wait_observation -- --list)
for name in stale_running_cli stale_running_reference stale_running_mcp stale_running_http live_quiet_job finite_budget delayed_terminal concurrent_observers; do
  require_test "$name"
done
listing=$(cargo test --lib -- --list)
for name in wait::tests::wait_observation_streak_resets wait::tests::wait_observation_threshold_boundaries wait::tests::wait_observation_final_read_precedence process::tests::wait_observation_errno_classification process::tests::wait_observation_invalid_pid; do
  require_test "$name"
done
cargo test --lib wait_observation -- --include-ignored
cargo test --test wait_observation -- --include-ignored
