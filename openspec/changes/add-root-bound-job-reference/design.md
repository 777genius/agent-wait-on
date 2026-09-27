# Fixed design

## Root producers
Preserve selection precedence. Absolutize in common resolve_root using std::path::absolute relative to caller cwd, not workload cwd. Do not canonicalize or append jobs. Preserve native platform semantics (Unix symlink/.. traversal; Windows absolute-path rules). Use the same resolved value for storage and response; missing roots remain valid creation destinations. Reject non-UTF8 roots rather than lossy-serializing new references.
Add root: String with serde(default) to RunData/CreateData and populate run/create/start/restart/serve builders. No persisted model shape changes; existing meta.root and list/gc outputs become absolute through shared resolution. Document Rust struct-literal caller impact.

## JSON input
wait accepts positional ID or --from-json FILE|- exclusively. Neither/both, or from-json plus explicit global --root anywhere, are usage errors exit 2. Bound file/stdin reads to 16 MiB plus one sentinel byte. Invalid/read-failed input returns JSON invalid_argument/retryable=false/exit 1 without echoing input.
Use existing serde_json::Value, duplicate keys last-wins. Require top-level object, ok:true, absolute nonempty UTF8 root and nonempty ID; bare two-field objects are rejected. Optional schema_version, when present, must be decimal MAJOR.MINOR with major 0. Ignore other fields; never execute them. Support MCP structuredContent or identical text JSON body, not JSON-RPC/tool wrappers.
Reject NUL in root/ID. Require ID to be exactly one native Component::Normal equal to original input; also reject slash, backslash and colon on all platforms. Thus . and .. are rejected; legacy ULID and current hex IDs work. No prefix resolution in this path.
Exact lookup MUST persist through actual wait open, not a preflight exact check followed by prefix-capable reopen. Preserve public WaitOpts if feasible through an internal exact-mode entry point sharing wait logic. Disappearance fails; never bind to a prefix. Environment/default roots are ignored. Preserve until/forever/poll; wait never kills jobs or writes job state.
This is a same-host/filesystem handoff, not a sandbox: trusted root contents/symlink mappings are assumed. No cross-host or malicious filesystem identity guarantee.

## Errors
Keep pub JobNotFound(String) and downcasting. Attach typed SearchedRoot context at lookup failure, including exact lookup. CLI/MCP construct message and details {root,job_id} explicitly from typed fields, not anyhow Display (CLI chains and MCP outermost differ). Preserve job_not_found, retryable=false, CLI exit 1, embedded/serve classification. Externally constructed root-less errors stay classifiable; omit unavailable root rather than inventing it.

## Schema/docs
Bump shared schema 0.3 to 0.4 under current minor policy. root optional in public response schema but always emitted on new launch success; serde(default) preserves old deserialization. Add error.details schema. Update schema constants/examples/tests/CHANGELOG/README/docs/skills/site and CLI/MCP descriptions. Do not change MCP wait input or tighten all MCP output schemas.

## Tests
Name new tests root_reference*. Real MCP launch structuredContent saved unchanged then CLI wait with cwd/environment B observes A; stdin parity; same ID in A/B; absent A never falls back B; parent root failure reports parent without searching children; missing exact ID never binds to matching prefixes. Invalid JSON/wrapper/ok:false/missing ok/root, relative root, NUL/path IDs/C:x, oversized input, malformed/incompatible schema and selector conflicts fail safely.
Cover legacy ULID and current ID; explicit/env/XDG/platform defaults with isolated home; relative root versus job cwd, spaces/non-ASCII, Unix symlink/.. and platform-gated Windows paths. CLI/MCP errors agree; embedded downcast remains. Cover all launch builders including serve, running/terminal/notification paths, old model deserialization, existing prefix/ambiguous/until/forever behavior and schema/docs checks.

## Review resolution
Prior independent Fable review required typed root-aware adapter conversion and safe component validation. Both are mandatory above. Additional boundary decisions fixed here: 16 MiB cap, ok required, no bare two-field inputs, and exact lookup through actual wait operation.
