# agent-wait-on 0.3.0

Initial downstream release based on agent-exec 0.2.40. Original history and MIT
copyrights are retained; see UPSTREAM.md.

- Primary `agent-wait-on` binary and compatibility `agent-exec` binary.
- Native universal plugin packages, bundled MCP server, and waiting skill.
- Verified platform installers delegating client registration to UAP.
- Bounded stream framing and command event delivery, without changing managed
  workload termination or the schema 0.5 result contract.

Raw per-stream logs retain their bytes. Combined logs and output-match event
matching use frames of at most 64 KiB of raw input, so a regex spanning frame boundaries is
not guaranteed to match. Failed raw log writes are not yet surfaced as a
dedicated capture-health result.

On Unix, timed-out event commands are terminated as an owned process group.
On Windows, cleanup terminates and reaps its owned shell child; descendants
may survive. A descendant retaining inherited stdin can keep the detached
writer alive until the pipe closes or the supervisor exits. This cleanup never
terminates the workload's Job Object. Unix descendants escaping the owned
process group are likewise outside the cleanup guarantee.

MCP observations are capped at 50 seconds in the portable package. For long
waits, use CLI `wait --forever` within one host tool execution session. Native
clients may require a fresh session or reload for newly installed tools.

Next adapters: exact-commit CI result normalization, hosted-job status and JSON
conditions. MCP Events remains an optional delivery integration to evaluate.
