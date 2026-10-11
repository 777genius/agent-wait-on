# Upstream

This is an independently maintained downstream of
[tumf/agent-exec](https://github.com/tumf/agent-exec), distributed under MIT.
The original copyright, license and commit authors are preserved.

Imported release: **v0.2.40**

Imported commit: `b0d82ecd12bd449f63e0eecf4f7e832cefbfc46f`

Remotes in the development checkout:

- `origin`: https://github.com/777genius/agent-wait-on.git
- `upstream`: https://github.com/tumf/agent-exec.git

Remote configuration is local. Restore it in a fresh clone with:

```sh
git remote add upstream https://github.com/tumf/agent-exec.git
git fetch upstream --tags
```

Integrate reviewed upstream changes on a separate branch, preserving ancestry.
Do not mirror-push over our changes or rewrite upstream author identities.
Use our own release versions. Keep the `agent_exec` Rust library name, the
`agent-exec` binary alias, persisted schema and `AGENT_EXEC_*` variables
compatible until a deliberate migration is accepted.

The original documentation is retained at `docs/UPSTREAM_README.md`; it describes
the original project and its distribution channels, not this downstream release.
