# SCHEMAS (design pack)

Reference schemas shipped with the design pack. The implementation repo (`agenium-app/schemas/`) holds the generated/maintained copies:

| Here | In the repo | Notes |
|---|---|---|
| `*.schema.json`, `tools/` | `schemas/ref/` (+ generated `schemas/*.schema.json`) | tool and entity JSON Schemas |
| `db-core.surql`, `db-agent.surql` | `schemas/ref/` | SurrealQL: reference for the **optional** `surrealkv` backend (ADR-49) |
| `sql/001_core.sql`, `sql/agent_001.sql`, `sql/agent_002.sql` | `schemas/sql/` = `crates/agenium-store/schema/sqlite/` | **primary store** (SQLite) migrations, mirrored by `cargo xtask schemas` |
| `sql/drafts/agent_003_graph_memory.sql` | `schemas/sql/drafts/` | **DRAFT, not applied** (ADR-51 graph memory); becomes migration `agent_003` in M1.5 |

Note: `db-core.surql` / `db-agent.surql` here are the *design-pack originals*. The optional SurrealDB backend actually uses the patched copies in `crates/agenium-store/schema/surreal/` (patch recorded in `S1-schema-patch.diff`, plus `extensions.surql`); the two sets intentionally differ.
