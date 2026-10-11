# agent-wait-on 0.3.0

Initial downstream release based on agent-exec 0.2.40. Original history and MIT
copyrights are retained; see UPSTREAM.md.

- Primary `agent-wait-on` binary and compatibility `agent-exec` binary.
- Native universal plugin packages, bundled MCP server, and waiting skill.
- Four release platforms: macOS arm64/amd64 and Linux arm64/amd64 (glibc).
- Verified shell installer delegating client registration to UAP.
- Unix launch waits for the workload PID handoff, preventing an immediate
  `kill` from targeting the temporary supervisor PID and leaving stale state.
- Bounded stream framing and command event delivery, without changing managed
  workload termination or the schema 0.5 result contract.

Raw per-stream logs retain their bytes. Combined logs and output-match event
matching use frames of at most 64 KiB of raw input, so a regex spanning frame boundaries is
not guaranteed to match. Failed raw log writes are not yet surfaced as a
dedicated capture-health result.

Windows is not release-supported in 0.3.0; follow the
[Windows port](https://github.com/777genius/agent-wait-on/issues/4). Inherited Windows runtime tests
expose lock access and platform-specific fixture failures that need separate
porting work. The PowerShell installer and Windows package builder remain
development prototypes and are not published as release assets.

On Unix, timed-out event commands are terminated as an owned process group.
Descendants escaping that group are outside the cleanup guarantee.

MCP observations are capped at 50 seconds in the portable package. For long
waits, use CLI `wait --forever` within one host tool execution session. Native
clients may require a fresh session or reload for newly installed tools.

Next adapters: exact-commit CI result normalization, hosted-job status and JSON
conditions. MCP Events remains an optional delivery integration to evaluate.
