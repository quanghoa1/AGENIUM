# ADR-120: Installing on the user's machine asks the user; the user's decisions reach later tickets; tickets naming the same file run in order

Status: Accepted (2026-10-27; owner: "Việc dùng powershell khi không có python thì liệu có cho agent cài đặt không … Plan
bị chồng thì nên kiểm tra nếu trùng file thì … follow up tức làm agn1 trước rồi làm agn2"). Amends ADR-64/117
(command gate) and decision 18 (overlapping work). Found on the clean Windows VM run of 2026-10-27: no Python there;
the agent asked on AGN-1, the user said "use PowerShell", and AGN-2 asked the same; AGN-1 and AGN-2 both wrote
`deliverables/check.py`.

## Decision

1. **Installs.** A command that installs on the user's machine — a system package manager (winget, choco, scoop, apt,
   dnf, brew, snap …), `sudo`, an MSI or a PowerShell module, a global package (`npm -g`, `yarn global`, `dotnet tool
   -g`), `cargo/go/gem install`, `pip` outside a virtual environment, an installer script piped to a shell — goes to
   the user under every policy, sandbox or not, and also when the user chose to run without a sandbox (ADR-117).
   Dependencies inside the worktree (a venv, `node_modules`, `pip --target`) do not. The work order says so: a missing
   tool is asked about, not worked around silently. (`runtime::bridge::system_install`.)
2. **The user's decisions.** A ticket's first work order lists the user's latest answers on the project's other
   tickets (up to five, with the question each answers), as decisions that hold for this ticket too.
3. **Overlap.** A ticket that declares no `scope_paths` is read for the files it names in its title and acceptance
   criteria (a path with a folder and an extension, or a backticked file name). Two open tickets without a dependency
   path that change the same file are refused at creation with the choices: depend on the other (it starts once that
   one is merged, from its work), put the work in one ticket, or declare `scope_paths` when they change different
   files. Different files stay parallel; the merge queue and its conflict reopen remain the safety net.
4. **Early rebase.** After a merge, every ticket in progress whose branch or working tree changes a merged file gets a
   system comment at once to rebase on main (decision 18, not done until now).

Following up (order) was preferred to merging parallel worktrees for the same file: the later ticket builds on the
earlier result instead of writing it twice and resolving the conflict afterwards.

## Verified on the clean Windows VM (2026-10-27, DeepSeek via OpenCode Go, running without a sandbox)

The same request as before: the plan's later tickets that named `deliverables/check.py` were made to depend on the
first (AGN-2 after AGN-1, AGN-3 after both); the agent asked about the missing Python once, on AGN-1; AGN-2 and AGN-3
did not ask again; 51 shell commands ran, all succeeded; all three tickets closed. The review of AGN-3 still doubted
the PowerShell substitute and tried to ask (refused: a review cannot ask), so the user's decisions now reach review
work orders too.

## Consequences

- `agenium_core::graph::paths_named`; `engine::effective_scope`; `comments::user_answers`; `Runtime::user_decisions`;
  `Effects::nudge_rebase`.
- Tests: `bridge.rs::installs_on_the_machine_are_told_from_project_dependencies`,
  `runtime.rs::sandboxed_commands_run_without_asking` (an install asks under unsandboxed), `runtime.rs::
  denied_command_question_and_continuation` (AGN-2 starts with AGN-1's answer), `lifecycle.rs::
  tickets_naming_the_same_file_run_one_after_the_other`, `lifecycle.rs::a_merge_nudges_work_in_progress_on_the_same_file_to_rebase`,
  `graph.rs::files_named_by_a_ticket`.
