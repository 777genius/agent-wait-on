## Implementation Tasks

- [ ] Absolutize roots and populate all launch response builders while preserving old deserialization (verification: integration - cargo test root_reference; tests/integration.rs producer/path tests; verification-id: root-reference).
- [ ] Implement bounded JSON file/stdin input, selector validation and actual exact-mode wait without fallback (verification: integration - cargo test root_reference; tests/integration.rs input/collision tests; verification-id: root-reference).
- [ ] Attach typed root context and align CLI/MCP error rendering, preserving embedded/serve classification (verification: unit - cargo test root_reference; tests/integration.rs error/downcast tests; verification-id: root-reference).
- [ ] Add real MCP-to-CLI execution tests and positional-ID/observation regressions (verification: e2e - cargo test root_reference; tests/integration.rs handoff tests; verification-id: root-reference).
- [ ] Update schema 0.4, response/error definitions, CLI/MCP/skill documentation and contract regression assertions (verification: integration - cargo test root_reference; tests/integration.rs schema and documented version tests; verification-id: root-reference).

## Final Validation
Archive validation is authoritative: cflx openspec validate add-root-bound-job-reference --archive-gate. Verification files stay in source/tests, not active-change paths. Acceptance runs repository regression checks described in proposal.md. No push/publication/installation.
