//! The work order rendered as the CLI's prompt, and the CLI's own home.

use super::*;

/// The work order as one prompt: the CLI keeps its own system prompt, so Agenium's goes first as instructions, with how
/// the run works: every CLI works through Agenium's tools over MCP (ADR-117, ADR-129).
/// `subagents`: a Solo agent's run, which may start the CLI's own subagents (they work through the same tools).
pub fn render(order: &WorkOrder, protocol: &[String], subagents: bool) -> String {
    let mut s = String::from("<agenium_instructions>\n");
    for b in &order.system {
        s.push_str(b.text.trim_end());
        s.push_str("\n\n");
    }
    let protocol = protocol.join(", ");
    let how = format!(
        "How this run works: you are an Agenium agent running in your own harness, and you work through the tools of \
the MCP server \"{MCP_SERVER}\", not your own: `read_file`, `list_dir`, `glob`, `grep` and `find_code` to read; \
`edit_file`, `multi_edit`, `apply_patch` and `write_file` to change files; `shell` for every command (tests, builds, \
scripts), with `background` for a server or watcher you start to check your work (`shell_output` reads it, \
`shell_stop` ends it); `todo` for your task list; `fetch` for a web page; `view_image` for an image. Use those this run \
offers. They keep the ticket's write scope and run commands under Agenium's rules without asking anyone. Your own \
shell is turned off, and your own file and search tools reach only your scratch folder, not the ticket worktree. \
Tools named above that are not in that list map to Agenium's equivalents. The protocol tools ({protocol}) come from \
the same server.\n"
    );
    s.push_str(&how);
    if subagents {
        s.push_str(
            "You are a Solo agent: you may start your own subagents with your harness's subagent tool for work that \
can run in parallel. That tool stays on: call it directly (a list of tools taken from inside a script may not show it), \
and tell each subagent to work through the tools of the same MCP server.\n",
        );
    }
    s.push_str("</agenium_instructions>\n");
    for m in &order.messages {
        let who = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        for p in &m.parts {
            match p {
                Part::Text { text } => s.push_str(&format!("\n[{who}]\n{text}\n")),
                Part::ToolCall { name, args, .. } => {
                    s.push_str(&format!("\n[{who} called {name}] {args}\n"))
                }
                Part::ToolResult {
                    name,
                    content,
                    is_error,
                    ..
                } => s.push_str(&format!(
                    "\n[result of {name}{}]\n{content}\n",
                    if *is_error { ", error" } else { "" }
                )),
                Part::Thinking { .. } | Part::Image { .. } => {}
            }
        }
    }
    s
}

/// The driver's environment with the user's home replaced by the per-run home, unless the driver set it: a CLI that
/// looks for skills, rules or settings under the home (`~/.agents`, `~/.config/...`) finds only what Agenium put
/// there, never the user's (their files are not touched; their own CLI keeps using them).
pub fn isolated_home(
    home: &std::path::Path,
    mut env: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    for k in ["HOME", "USERPROFILE"] {
        env.entry(k.to_owned())
            .or_insert_with(|| home.display().to_string());
    }
    env
}

/// The mark of a relayed user message in a CLI run: the API loop's, with the run's tag.
pub(super) fn steered_mark(tag: &str) -> String {
    format!(
        "{} · {tag}]",
        agenium_backend::STEERED.trim_end().trim_end_matches(']')
    )
}
