//! The work order rendered as the CLI's prompt, and the CLI's own home.

use super::*;

/// The work order as one prompt: the CLI keeps its own system prompt, so Agenium's goes first as instructions.
/// How a run's commands and files are handled, as the CLI is told (ADR-117).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commands {
    /// The CLI runs inside Agenium's sandbox (its own tools do the work).
    Contained,
    /// A CLI that runs contained elsewhere runs on the host: no sandbox works here and the user accepted that.
    Unsandboxed,
    /// A hooked CLI: Agenium's tools over MCP do the work; its own shell is refused.
    AgeniumTools,
}

pub fn render(order: &WorkOrder, protocol: &[String], commands: Commands) -> String {
    let mut s = String::from("<agenium_instructions>\n");
    for b in &order.system {
        s.push_str(b.text.trim_end());
        s.push_str("\n\n");
    }
    let protocol = protocol.join(", ");
    let how = match commands {
        Commands::Contained | Commands::Unsandboxed => format!(
            "How this run works: you are an Agenium agent running in your own harness. Do the work with your own \
file, search and shell tools inside the current directory (the ticket worktree); tools named above that you do not \
have map to your own equivalents. Agenium's protocol tools ({protocol}) come from the MCP server \"{MCP_SERVER}\": \
call them there. {}\n",
            if commands == Commands::Contained {
                "Shell commands run inside Agenium's sandbox: only the worktree is writable."
            } else {
                "This machine has no sandbox: your commands run directly on the user's machine, in the ticket \
worktree. Keep every change and every command inside it."
            }
        ),
        Commands::AgeniumTools => format!(
            "How this run works: you are an Agenium agent running in your own harness, and you work through the \
tools of the MCP server \"{MCP_SERVER}\", not your own: `read_file`, `list_dir`, `glob` and `grep` to read; \
`edit_file`, `multi_edit`, `apply_patch` and `write_file` to change files; `shell` for every command (tests, builds, \
scripts, a server you start to check your work). Use those this run offers. They keep the ticket's write scope and \
run commands under Agenium's rules without asking anyone. Your own shell tool is turned off; your own file tools still \
work inside the current directory (the ticket worktree), but use Agenium's. Files outside the worktree and your own \
web tools are refused. Tools named above that are not in that list map to Agenium's equivalents. The protocol tools \
({protocol}) come from the same server.\n"
        ),
    };
    s.push_str(&how);
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
