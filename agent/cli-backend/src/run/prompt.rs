//! The work order rendered as the CLI's prompt, and the CLI's own home.

use super::*;

/// The Agenium tools a run offers, said by what they are for; a run without write tools is said to change nothing.
fn offered(tools: &[String]) -> String {
    let has = |n: &str| tools.iter().any(|t| t == n);
    let list = |names: &[&str]| {
        let v: Vec<String> = names
            .iter()
            .filter(|n| has(n))
            .map(|n| format!("`{n}`"))
            .collect();
        match v.as_slice() {
            [] => String::new(),
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        }
    };
    let mut parts = Vec::new();
    let read = list(&["read_file", "list_dir", "glob", "grep", "find_code"]);
    if !read.is_empty() {
        parts.push(format!("{read} to read"));
    }
    let change = list(&["edit_file", "multi_edit", "apply_patch", "write_file"]);
    if !change.is_empty() {
        parts.push(format!("{change} to change files"));
    }
    if has("shell") {
        parts.push(if has("shell_output") {
            "`shell` for every command (tests, builds, scripts), with `background` for a server or watcher you start \
to check your work (`shell_output` reads it, `shell_stop` ends it)"
                .to_owned()
        } else {
            "`shell` for every command (tests, builds, scripts)".to_owned()
        });
    }
    for (name, what) in [
        ("todo", "your task list"),
        ("fetch", "a web page"),
        ("view_image", "an image"),
        ("browser", "your app as a user sees it, and its screenshots"),
    ] {
        if has(name) {
            parts.push(format!("`{name}` for {what}"));
        }
    }
    const KNOWN: [&str; 16] = [
        "read_file",
        "list_dir",
        "glob",
        "grep",
        "find_code",
        "edit_file",
        "multi_edit",
        "apply_patch",
        "write_file",
        "shell",
        "shell_output",
        "shell_stop",
        "todo",
        "fetch",
        "view_image",
        "browser",
    ];
    let others: Vec<String> = tools
        .iter()
        .filter(|t| !KNOWN.contains(&t.as_str()))
        .map(|t| format!("`{t}`"))
        .collect();
    if !others.is_empty() {
        parts.push(format!("and your MCP tools {}", others.join(", ")));
    }
    let mut out = if parts.is_empty() {
        "none in this run".to_owned()
    } else {
        parts.join("; ")
    };
    if change.is_empty() {
        out.push_str(". This run changes no files: it has no write tools");
    }
    out
}

/// The work order as one prompt: the CLI keeps its own system prompt, so Agenium's goes first as instructions, with how
/// the run works: every CLI works through Agenium's tools over MCP (ADR-117, ADR-129). `tools`: the Agenium tools this
/// run offers, the only ones named (a chat run told of tools it did not have took it for a contradiction).
/// `subagents`: a Solo agent's run, which may start the CLI's own subagents (they work through the same tools).
pub fn render(order: &WorkOrder, protocol: &[String], tools: &[String], subagents: bool) -> String {
    let mut s = String::from("<agenium_instructions>\n");
    for b in &order.system {
        s.push_str(b.text.trim_end());
        s.push_str("\n\n");
    }
    let protocol = protocol.join(", ");
    let how = format!(
        "How this run works: you are an Agenium agent running in your own harness, and you work through the tools of \
the MCP server \"{MCP_SERVER}\", not your own: {}. They keep the ticket's write scope and run commands under \
Agenium's rules without asking anyone. Your own shell is turned off, and your own file and search tools reach only \
your scratch folder, not the ticket worktree. Tools named above that are not in that list map to Agenium's \
equivalents. The protocol tools ({protocol}) come from the same server.\n",
        offered(tools)
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
    // The git a CLI runs for itself reads no global or system configuration: a `.gitconfig` a model planted in the
    // home (an alias, `core.fsmonitor`) would run a command outside every gate.
    env.entry("GIT_CONFIG_GLOBAL".to_owned())
        .or_insert_with(|| if cfg!(windows) { "NUL" } else { "/dev/null" }.to_owned());
    env.entry("GIT_CONFIG_NOSYSTEM".to_owned())
        .or_insert_with(|| "1".to_owned());
    env
}

/// The mark of a relayed user message in a CLI run: the API loop's, with the run's tag.
pub(super) fn steered_mark(tag: &str) -> String {
    format!(
        "{} · {tag}]",
        agenium_backend::STEERED.trim_end().trim_end_matches(']')
    )
}
