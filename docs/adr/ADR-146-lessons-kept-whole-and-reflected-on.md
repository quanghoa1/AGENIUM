# ADR-146: Lessons kept whole, proven lessons reflected on, an agent's own skills improved

Status: Accepted (2026-10-30). Owner, on the memory chains' weak points ("agents rarely write a skill; no skill update
from real experience; the settling pass never ran on real data; Codex drew five near-identical lessons"): "cải thiện
được không? tại sao lại rút 5 bài học gần giống nhau và các phần khác nữa".

## Context

Real memory chains (`--memory-reuse on --chain`: ticket 1 fails its pre-merge check once by design, tickets 2–4
follow) on DeepSeek v4.1 flash and Codex gpt-6-luna showed:

- **Fragmented lessons.** The decision call saw the earlier lesson and still chose ADD for each new angle of the same
  rule (wider, one more step): Codex ended with five lessons on one rule. The prompt said only "ADD (new information)".
- **A too-narrow lesson was not blamed.** Codex's first lesson ("under deliverables/ or reports/") made ticket 3 (a
  document elsewhere) fail; the extraction listed nothing as misleading — it was asked only for memories that "led to"
  the rejected work.
- **A clean ticket deleted a proven lesson.** A ticket closed without any signal DELETEd the lesson that had kept the
  earlier tickets right (its evidence guard asked only for a trusted new fact); the next ticket, no longer reminded,
  was reopened.
- **Skill writing waited for the agent to think of it.** In the learning loop the skill tools were used in 1 run of 6;
  in the chains, lessons held in three tickets and no skill was written.
- **An agent could not improve a skill it had made.** `propose_new_skill` refused a second skill on a covered subject
  and sent the agent to `propose_skill_update`, which refused the agent's own skills (only library and role skills
  have overlays).
- **The settling pass (ADR-145) never ran**: it waited for 20 new facts; four tickets learn about ten.

## Decision

1. **One lesson per rule.** The decision prompt: ADD only for something no existing memory covers; a lesson that
   widens, narrows, adds a step to or restates an existing lesson about the same rule is an UPDATE of it (the merged
   text keeps what holds of both). An UPDATE of a lesson keeps its kind, cue and the contexts it held in (as before).
   The extraction prompt: a memory that was followed but too narrow or incomplete counts as misleading.
2. **A clean ticket does not disprove a lesson.** A DELETE of a lesson by an episode without any signal (reopen, failed
   merge, correction) is refused; refining it is an UPDATE.
3. **Reflection on a proven lesson** (`runtime/reflect.rs`): after a ticket is learned from, each lesson that now holds
   in two contexts and was not reflected on (at most two an episode) is put to the agent's own side model (its API
   model, or a CLI agent's `[cli]` fallback): update a skill of its own set, make a new skill, or keep it as a memory.
   What it proposes goes through the gates of a proposal made in a run (`skill_propose`, shared by both): a trusted
   lesson held in two closed tickets, the shape, one subject, no injected instructions or secrets, the judge's
   contradiction read. Its tokens count as a `Memory` run; the outcome is in the ticket run's log.
4. **An agent's own skills are improved in place** (`skill_revise.rs`): `propose_skill_update` on a skill the agent
   created rewrites it in its own folder under the overlay's gates, recording the lesson as a revision.
5. **Settling** (ADR-145): the free pass (vectors and words) after every learned episode; the judge every 10 new facts.
6. A model's JSON answer with raw line breaks inside a string (a skill body) is read (`pipeline::json_object`).

## Verified (2026-10-30)

| Memory chain | Before | After |
|---|---|---|
| DeepSeek (two runs after) | lessons 2 → 0 after ticket 3 (a clean ticket deleted it) and ticket 4 reopened | 1 reopen (ticket 1, by design) in each; 2 lessons, held in 1 → 2 → 3 tickets; a skill made at ticket 3 (`docs-owner-first-line`, `docs-owner-header-check`) and revised with the other lesson, and in one run again at ticket 4 when the lesson widened to handbook/ and minutes/ |
| Codex | 5 lessons on one rule; the narrow one not blamed | 1 lesson; the narrow ones listed as misleading at ticket 3 and rewritten; a skill made at ticket 4 (`docs-team-owner-line`) |

Every reflection's proposal was accepted by the gates. Learning loop (three lessons held in two closed tickets): PASS
on DeepSeek ×2, Codex and Claude Code, 0 violations, the system skill unchanged; both DeepSeek runs updated their own
`commit-and-pr` and one made the procedure's skill (1 of 4 before); nothing was built from the fact.

Tests: `a_clean_episode_does_not_delete_a_lesson`; `a_proven_lesson_is_reflected_on_once_through_the_gates` (refused
by the gates when the contexts are not closed tickets, logged, not asked again); `an_agent_creates_a_skill_only_through
_every_gate` (its own skill revised from a later proven lesson, refused with one context or injected words);
`parse::tests::raw_line_breaks_inside_strings_are_read`; `the_judge_reads_every_ten_new_facts`.

## Consequences

- A reflection costs one or two side calls per proven lesson (about 2,000–6,000 tokens), once.
- CLI agents reflect only with a `[cli]` fallback API model, as they extract memory only with one.
- In the chains the settling pass ran after every episode and found nothing to fold or flag (no paraphrases or changed
  figures); the judge did not run (no key in the eval).
