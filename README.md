# agent-wait-on

Run a command once. Keep its job reference. Wait for the result without making
an agent repeatedly check status.

Native Rust CLI and local MCP server, with a [Universal Agent Plugins](https://github.com/777genius/universal-agent-plugins)
package for Codex CLI, Codex Desktop, and Claude Code. Independent downstream
of [tumf/agent-exec](https://github.com/tumf/agent-exec), preserving its history,
MIT license, managed process lifecycle, and JSON contracts. See [UPSTREAM.md](UPSTREAM.md).

## Install the agent plugin

macOS or Linux:

```sh
curl -fsSL https://github.com/777genius/agent-wait-on/releases/download/v0.3.0/install.sh -o /tmp/agent-wait-on-install.sh
sh /tmp/agent-wait-on-install.sh --target codex,claude
```

Windows PowerShell:

```powershell
Invoke-WebRequest https://github.com/777genius/agent-wait-on/releases/download/v0.3.0/install.ps1 -OutFile "$env:TEMP/agent-wait-on-install.ps1"
& "$env:TEMP/agent-wait-on-install.ps1" --target codex,claude
```

Select only `codex` or `claude` when appropriate; omit `--target` for UAP's
client discovery. Install [UAP](https://github.com/777genius/universal-agent-plugins)
first, or have Node.js/npm available so the installer can invoke pinned
`universal-agent-plugins@0.1.80` through `npx`. Native binaries are supplied for
macOS arm64/amd64, Linux arm64/amd64 (glibc), and Windows amd64.

The installer verifies the release ZIP's SHA256 and retains an immutable
versioned package source, then delegates validation and client registration to
UAP. It does not install a global binary on `PATH`. The printed package path
contains `bin/agent-wait-on` (`.exe` on Windows). Job data is kept separately
under UAP's `PLUGIN_DATA`, and survives an observation timeout or MCP disconnect.

Codex CLI and Codex Desktop use the same Codex plugin configuration when they
use the same `CODEX_HOME`. Start a fresh session or reload the client after
installation. An already running chat may retain its old tool catalog. If UAP
reports prepared/manual activation, finish the displayed native client steps;
preparation alone is not proof of activation.

The Git repository's `portable-package/` is a build template. It intentionally
does not contain checked-in platform binaries. Use a release ZIP or the installer
above rather than installing that Git subdirectory directly.

## What agents get

- MCP tools to run, inspect, tail, wait for, and explicitly terminate managed jobs.
- Structured job results with an absolute `root` and `job_id`, captured output,
  and terminal state/exit code.
- A skill explaining bounded MCP observation, one persistent CLI waiter, and
  GitHub CLI watchers tied to the correct commit.

Launch normally observes up to 10 seconds, catching quick failures in the first
response. MCP `wait` normally observes up to 30 seconds. This plugin caps a
single MCP observation at 50 seconds to fit common 60-second client limits.
`until` limits waiting, not execution: the workload continues afterward.
A transport timeout means the result was not observed; it does not justify
launching the workload again.

For a long wait in the current agent turn, keep one host execution session:

```sh
/absolute/package/bin/agent-wait-on --root /absolute/jobs wait --forever JOB_ID
```

The MCP server's initialization instructions report its installed executable
path, and the installer prints the package source path. Resolve the binary path
before using it; arbitrary skill Markdown does not expand `PLUGIN_ROOT`.
The host execution tool still controls its own deadline and session handling.
This plugin never starts or resumes an agent turn automatically.

For CI, run one official watcher as a managed job, for example:

```sh
gh run watch RUN_ID --repo OWNER/REPO --exit-status --interval 30
gh pr checks PR --repo OWNER/REPO --watch --interval 30
```

`gh` handles API polling in its process. The agent waits for that process and
verifies the expected checks and exact commit before accepting the result.
There is no dedicated GitHub webhook or JSON predicate adapter in this release.
MCP Events is a future optional transport, not required for reliable waiting.

## CLI and compatibility

Build from source with stable Rust:

```sh
cargo install --git https://github.com/777genius/agent-wait-on --tag v0.3.0 --bin agent-wait-on --locked
agent-wait-on run --forever -- cargo test
agent-wait-on schema
```

The `agent-exec` compatibility binary remains buildable, the library crate remains
`agent_exec`, and existing `AGENT_EXEC_*` settings and `agent-exec.schema.json`
are retained. [The upstream command reference](docs/UPSTREAM_README.md) describes
the inherited API; substitute the primary executable name `agent-wait-on`.

Output framing is bounded at 64 KiB. Raw stdout/stderr logs retain original
bytes; combined logs and output-match events may split an exceptionally long
line into frames. Event-command delivery has a bounded deadline. Review
[release notes](RELEASE_NOTES.md) for platform-specific limitations.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cd scripts
npm ci --ignore-scripts
npm run typecheck
npm test
```

Rust/installer CI runs on Linux, macOS, and Windows. Release CI builds all five
native packages, smoke-tests packaged binaries, and creates a draft containing
ZIPs, installers, and SHA256SUMS. Publication follows artifact and installed
package verification. Keep client tests in disposable isolated profiles.
