---
change_type: implementation
priority: high
dependencies: []
verifications:
  - id: root-reference
    requirement: Root-bound CLI and MCP handoff and diagnostics
    phase: pre-integration
    owner: conflux-acceptance
    trigger: pull-request-validation
    automation: tests/integration.rs
    evidence: cargo test root_reference
    rerun: cargo test root_reference
    prerequisites: []
    execution_class: repository-local
    completion_role: change-blocking
---
# Root-bound job references

**Change Type**: implementation

## Context
Launch results expose IDs/logs but not the resolved jobs root. Manually transferring IDs between MCP and CLI can select a parent directory and fail without showing the searched root.

## Solution and boundary
Implement design.md: absolute root in launch results, wait --from-json FILE|- and root-aware errors. These form one reference producer/consumer/failure contract with a single schema 0.4 transition. Touch jobstore, response builders including serve, schema, CLI wait and MCP error adapters, tests and existing contract documentation/skills. No independent cleanup.

## Acceptance Criteria
Actual MCP launch JSON saved unchanged is consumable by CLI wait despite conflicting cwd/environment. Only its exact ID and root are used. Missing jobs report ID and searched root without fallback. Existing observation, notifications, positional-ID prefix lookup and error classification remain supported.

## Explicit Completion Conditions
Passing root_reference tests must exercise real MCP/CLI jobs, not just parser fixtures. All producer paths and invalid-input paths are covered. Schema/docs agree. Archive promotes spec deltas; test paths remain usable after archive. No installation is claimed.

## Verification
New tests use root_reference names so cargo test root_reference covers unit/integration/MCP behavior. prek.toml filters Rust filenames and is not unconditional. Acceptance also runs cargo fmt --all -- --check, cargo clippy --all-targets --all-features -- -D warnings and cargo test --all once for regression; no repeated stability loop.

## Out of Scope
No root discovery, fallback, URI tokens, log-path inference, shell evaluation, cross-host authentication, arbitrary MCP root inputs, dependencies, installation, tags, push or publication. Preserve .cflx.jsonc and its local version/index hook.
