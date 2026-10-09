---
change_type: implementation
priority: high
dependencies: []
verifications:
  - id: wait-observation
    requirement: Read-only wait observation inconsistency across CLI, reference, MCP and HTTP
    phase: pre-integration
    owner: conflux-acceptance
    trigger: pull-request-validation
    automation: scripts/verify-wait-observation.sh
    evidence: sh scripts/verify-wait-observation.sh
    rerun: sh scripts/verify-wait-observation.sh
    prerequisites: []
    execution_class: repository-local
    completion_role: change-blocking
---
# Bound stale-running wait without inventing an outcome

**Change Type**: implementation

## Context
The shared wait loop observes only persisted state and log drain. A disappeared workload with stale running state can leave every waiter blocked indefinitely. Process absence is not an authoritative completion result. The historical disappearance cause is unknown and is not this implementation's acceptance question.

## Solution
Implement the exact read-only policy in design.md, enabled by default for Unix wait consumers. Return a typed observation error after sustained explicit ESRCH; preserve the job and allow later recovery of its real result. Cover positional and root-bound CLI, MCP wait and HTTP wait together. Preserve launch observation and status semantics.

## Acceptance and Completion Conditions
All mandatory causal tests named in design.md must run and pass via the tracked fail-closed verification entrypoint. CLI emits one error JSON and exits 1; HTTP returns 409; MCP returns the same domain envelope. No waiter mutates any job file or sends a termination signal. Ordinary low-output jobs and delayed terminal saving retain their authoritative result. Existing root binding, deadlines and log drain remain valid. Schema 0.5 and documentation describe the new observation error as distinct from job failure.

## Scope
src/wait.rs, src/process.rs, src/main.rs, src/mcp.rs, src/serve.rs, src/schema.rs, tests/wait_observation.rs, focused unit tests in existing modules, scripts/verify-wait-observation.sh, README.md, skills/agent-exec/SKILL.md, skills/agent-exec/references/cli-contract.md and skills/agent-exec/references/completion-events.md (including the schema changelog). The schema bump additionally permits only the expected-version literal update from 0.4 to 0.5 in tests/support/mod.rs::assert_envelope and tests/integration.rs::all_commands_use_schema_version_0_2; do not change unrelated tests or historical version-migration examples. No dependencies or new daemon. Preserve .cflx.jsonc and the local-only per-change version hook. Local merge/archive is permitted; installation, live job repair, process shutdown/restart, tags, push, release and publication are excluded.
