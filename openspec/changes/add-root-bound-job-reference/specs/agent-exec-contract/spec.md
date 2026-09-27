## ADDED Requirements

### Requirement: Root-bound job reference handoff
Successful run/create/start/restart responses MUST include the absolute resolved jobs root. CLI wait MUST accept bounded successful result JSON from file/stdin, using only its exact job_id and root. Invalid references and conflicting explicit selectors MUST be rejected without fallback. Existing positional-ID semantics MUST remain supported.

#### Scenario: MCP launch consumed in another environment
- **WHEN** root A's MCP launch JSON body is passed unchanged to wait --from-json while cwd/environment select B
- **THEN** only the exact job in A is observed, even if B has the same ID
- **AND** absence in A fails without prefix matching or other-root lookup

#### Scenario: Invalid reference
- **WHEN** JSON is oversized, malformed, lacks ok:true/absolute root, contains unsafe IDs or conflicts with positional ID/explicit root
- **THEN** input is rejected without executing embedded fields or inferring roots

### Requirement: Searched root in missing-job diagnostics
CLI/MCP job_not_found errors from repository lookup MUST include actual searched root and requested ID in message and error.details while preserving code/retryability/exit semantics.

#### Scenario: Parent directory supplied
- **WHEN** a target exists under a jobs child but the caller selects its parent
- **THEN** lookup fails and reports the parent and ID without searching children

### Requirement: Additive root reference schema
Root references MUST use schema 0.4, preserve existing fields and old response deserialization, and document same-host scope and Rust struct-literal compatibility impact.

#### Scenario: New launch response
- **WHEN** a new successful launch is serialized
- **THEN** schema 0.4 and root are emitted, while public schema keeps root optional for compatibility
- **AND** old launch JSON remains deserializable and conventional wait ID works
