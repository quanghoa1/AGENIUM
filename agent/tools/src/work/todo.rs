//! `todo`: the run's task list.

use super::*;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TodoArgItem {
    pub text: String,
    pub status: TodoStatus,
}

/// Replace your checklist for this run.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TodoArgs {
    pub items: Vec<TodoArgItem>,
}

pub struct TodoTool;

#[async_trait]
impl Tool for TodoTool {
    fn name(&self) -> &'static str {
        "todo"
    }
    fn description(&self) -> &'static str {
        "Replace your checklist of steps for this run (shown to the user). Keep exactly one item in_progress."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<TodoArgs>()
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: TodoArgs = args(a)?;
        let items: Vec<TodoItem> = a
            .items
            .into_iter()
            .map(|i| TodoItem {
                text: i.text,
                status: i.status,
            })
            .collect();
        let done = items
            .iter()
            .filter(|i| i.status == TodoStatus::Done)
            .count();
        let rendered: Vec<String> = items
            .iter()
            .map(|i| {
                format!(
                    "[{}] {}",
                    match i.status {
                        TodoStatus::Done => "x",
                        TodoStatus::InProgress => ">",
                        TodoStatus::Pending => " ",
                    },
                    i.text
                )
            })
            .collect();
        let n = items.len();
        ctx.set_todo(items);
        Ok(ToolOutput::text(format!(
            "{done}/{n} done\n{}",
            rendered.join("\n")
        )))
    }
}
