# Context switch and chat memory, before and after ADR-155 (2026-11-02)

Produced by `quality/eval/examples/context_switch.rs` with OpenCode Go's free models (`beam-free`, 1M tokens; `iceman-contributor-free`, 262 144). `old-*` ran on commit c119d77 (before ADR-155), `v3-*` and `new-chat*` on the version of this commit. `work/tools` and `work/text` are the model switch with a history of tool results or of long writing; `chat` is four Lead turns with a learned preference in memory.
