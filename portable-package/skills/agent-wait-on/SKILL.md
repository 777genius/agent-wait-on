---
name: agent-wait-on
description: Run a command once, preserve its job reference, and wait for completion. Use for long builds, tests, and GitHub CI watchers instead of repeated agent status polling.
---

# Managed jobs and waiting

Use the `agent-wait-on` MCP server's `run`, `wait`, `status`, `tail`, and `kill`
tools. Inspect the client's exposed tool schemas for their exact names and
arguments. This plugin runs processes; it does not create or resume agent chats.

1. Call `run` once with a non-empty `command` argv array and an explicit `cwd`
   when needed. Example: `{"command":["cargo","test"],"cwd":"/absolute/project","until":10}`.
2. Preserve the successful response, especially `job_id` and its absolute
   `root`. A bounded response may leave the job running.
3. Call `wait` for that same `job_id` when completion is needed. The portable
   package caps MCP observation at 50 seconds to leave room within common
   60-second client request limits. A client can impose a shorter limit. Treat a
   transport timeout as unknown observation, then check the existing job; do not
   launch its workload again automatically.
4. Use `status` to inspect state and `tail` to recover output when necessary.
   Terminal state is not proof of success: inspect the process exit code and
   the workload's own result.

`until` limits observation only. The process keeps running when waiting ends.
`kill` intentionally signals the process. `abandon_job_after` terminates the job
and can lose unfinished results; it requires `acknowledge_result_loss=true`.
Never add abandonment merely to fit a tool request deadline. Removed `timeout`
and `timeout_ms` launch arguments are errors, not observation controls.

## Long waits in one CLI process

For a long completion wait, start one CLI waiter and keep its process/session
handle in the host execution tool until it completes:

```sh
"${PLUGIN_ROOT}/bin/agent-wait-on" wait --forever --from-json /absolute/launch-result.json
```

Save the successful launch response body verbatim to the JSON file: CLI stdout,
or the MCP `structuredContent` object / its identical text JSON body. Do not save
the outer JSON-RPC envelope or a `content` array.
`--from-json` selects its exact root and job, even from a different directory.
Without a saved response, use the returned root explicitly:

```sh
"${PLUGIN_ROOT}/bin/agent-wait-on" --root /absolute/jobs wait --forever JOB_ID
```

On Windows, use `& "$env:PLUGIN_ROOT/bin/agent-wait-on.exe"` with the same
arguments. Resolve `PLUGIN_ROOT` from the client-provided installed plugin root,
the executable path reported in the MCP server initialization instructions,
or the installed package location reported by the installer. The examples are
placeholders: UAP relocates MCP command/args/env fields, but does not expand
arbitrary skill Markdown or guarantee a shell environment variable. Use the
resolved absolute binary path if the host does not export `PLUGIN_ROOT`.
Do not invoke a bare binary name unless the user installed it on `PATH`.

The CLI waiter is independent of the MCP 50-second cap. A host tool may yield a
session handle during the wait; resume that handle, retaining one waiter. Do
not build an LLM loop that repeatedly launches `status`, `tail`, or `wait`.

## GitHub CI

Use official `gh` watcher commands as the managed workload:

```sh
gh pr checks PR --repo OWNER/REPO --watch --required --interval 30
gh run watch RUN_ID --repo OWNER/REPO --exit-status --interval 30
```

Before starting, record the current PR head SHA and the required check set, or
the run ID, attempt, and expected tested SHA for an Actions run. Run one watcher
through `run`, then use the same job reference to wait for its completion.
Before accepting green, re-read the current head/tested SHA and verify the full
expected set for that exact code. Missing checks, cancelled, skipped, neutral,
read failures, and observation timeouts are not success. If there are no required
checks, remove `--required` and define the expected set explicitly. Diagnose
failures with `gh`; restart only a necessary failed phase.

This package has no dedicated GitHub or generic JSON predicate adapter. `gh`
must already be available and authenticated in the command's environment.
