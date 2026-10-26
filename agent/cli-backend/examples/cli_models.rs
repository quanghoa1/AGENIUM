//! Prints what a vendor CLI reports about its models (no model turn, no quota): `codex` prints `model/list` as it
//! came, which carries each model's reasoning efforts.
//!
//! cargo run -p agenium-cli-backend --example cli_models -- codex <scratch dir>

use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scratch = PathBuf::from(
        args.get(1)
            .cloned()
            .unwrap_or_else(|| ".models-probe".into()),
    );
    match args.first().map(String::as_str) {
        Some("codex") => {
            let program = which("codex")?;
            let v = agenium_cli_backend::codex::model_list(&program, &scratch).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
            );
        }
        _ => return Err("usage: cli_models codex [scratch dir]".into()),
    }
    Ok(())
}

fn which(name: &str) -> Result<PathBuf, String> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .flat_map(|d| {
            [
                d.join(format!("{name}.exe")),
                d.join(format!("{name}.cmd")),
                d.join(name),
            ]
        })
        .find(|p| p.is_file())
        .ok_or_else(|| format!("{name} is not on PATH"))
}
