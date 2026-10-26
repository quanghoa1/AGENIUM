# ADR-47: Embedder trait; default model

- **Status:** Proposed — default NOT changed; needs owner confirmation
- **Date:** 2026-10-23 (Asia/Saigon)
- **Context / decision / consequences:**

- `agenium-memory::Embedder` is configurable (`memory.embedder`); `BruteForceIndex` (exact cosine) was the M0 default; **amended by ADR-50**: exact (sqlite-vec) below 100 000 facts per agent, usearch HNSW above (`ef_search` 128, scaled to 256 from 200 000 vectors). `memory.knn_ef` only applies to the optional SurrealDB backend.
- macOS spike: AllMiniLML6V2 recall@5 0.93 vs bge-small-en 0.87. Suggested default = AllMiniLML6V2; the config default stays as in the pack until the owner confirms (changing a decision needs a new ADR + approval).
- Real model loading (fastembed) is not in M0; `HashEmbedder` is a deterministic stand-in.

**Amendment (2026-10-26).** The local model is `bge-small-en-v1.5` (candle, pinned revision, sha256-checked). It is
installed with `agenium models download` or from the desktop app's Memory page. That page says when it is missing:
memory then recalls by shared words only (`HashEmbedder`). After a restart the app uses the model, and memories are
re-embedded on their next run.

