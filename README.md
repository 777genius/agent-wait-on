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

Windows support is deferred to [the Windows port](https://github.com/777genius/agent-wait-on/issues/4).
Version 0.3.0 does not provide a supported Windows
runtime or release installer. The repository retains the PowerShell installer
prototype and fixture checks for future Windows work; they do not establish
runtime support.

Select only `codex` or `claude` when appropriate; omit `--target` for UAP's
client discovery. Install [UAP](https://github.com/777genius/universal-agent-plugins)
first, or have Node.js/npm available so the installer can invoke pinned
`universal-agent-plugins@0.1.80` through `npx`. Native binaries are supplied for
macOS arm64/amd64 and Linux arm64/amd64 (glibc).

The installer verifies the release ZIP's SHA256 and retains an immutable
versioned package source, then delegates validation and client registration to
UAP. It does not install a global binary on `PATH`. The printed package path
contains `bin/agent-wait-on`. Job data is kept separately
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
If startup readiness cannot be observed after acknowledgement, the error
retains its job reference in `error.details`; inspect that job before taking
further action.

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
```

The `agent-exec` compatibility binary remains buildable, the library crate remains
`agent_exec`, and existing `AGENT_EXEC_*` settings and `agent-exec.schema.json`
are retained. Structured responses use schema version `0.5`.
[The upstream command reference](docs/UPSTREAM_README.md) describes
the inherited API; substitute the primary executable name `agent-wait-on`.
The release package bundles the schema file. A Cargo-only installation needs
`schema/agent-exec.schema.json` beside its binary to use the `schema` command.

Output framing is bounded at 64 KiB of raw input per frame. Raw stdout/stderr logs retain original
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

Rust CI runs on supported Linux and macOS hosts. Installer fixture checks also
cover the Windows prototype. Release CI builds all four supported
native packages, smoke-tests packaged binaries, and creates a draft containing
ZIPs, `install.sh`, and SHA256SUMS. Publication follows artifact and installed
package verification. Keep client tests in disposable isolated profiles.
