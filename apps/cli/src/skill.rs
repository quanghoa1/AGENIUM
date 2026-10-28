//! Skill library commands. Split out of `main.rs`.

use crate::*;

pub(crate) fn skill_owner(
    app: &App,
    agent: &Option<String>,
    role: &Option<String>,
) -> Result<agenium_skills::Owner> {
    Ok(match (agent, role) {
        (Some(a), _) => agenium_skills::Owner::Agent(app.agent_named(a)?.id.to_string()),
        (None, Some(r)) => {
            if !agenium_skills::valid_name(r) {
                bail!("invalid role id {r:?}");
            }
            agenium_skills::Owner::Role(r.clone())
        }
        (None, None) => agenium_skills::Owner::Library,
    })
}

/// Downloads an https .zip (at most the skill size limit) into a temporary file.
pub(crate) async fn download_zip(url: &str, dir: &Path) -> Result<PathBuf> {
    if !url.starts_with("https://") {
        bail!("only https URLs are accepted");
    }
    let mut resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    let path = dir.join("skill-download.zip");
    let mut f = std::fs::File::create(&path)?;
    let mut total = 0u64;
    while let Some(chunk) = resp.chunk().await? {
        total += chunk.len() as u64;
        if total > agenium_skills::MAX_TOTAL_BYTES {
            bail!(
                "the download is larger than {} MiB",
                agenium_skills::MAX_TOTAL_BYTES >> 20
            );
        }
        f.write_all(&chunk)?;
    }
    Ok(path)
}

pub(crate) async fn skill_cmd(cli: &Cli, sc: &SkillCmd) -> Result<()> {
    if let SkillCmd::Validate { dir } = sc {
        let s = agenium_skills::validate(dir, agenium_skills::Origin::Agent)?;
        println!(
            "ok: {} - {} ({} files)",
            s.meta.name,
            s.meta.description,
            s.files.len()
        );
        return Ok(());
    }
    let app = open(&root(cli)?)?;
    let reg = app.skill_registry();
    match sc {
        SkillCmd::List { agent } => {
            let a = app.agent_named(agent)?;
            let set = reg.for_agent(&a.id.to_string(), &a.role, &a.skills);
            if set.is_empty() {
                println!("(no skills)");
            }
            for s in &set.skills {
                let origin = serde_json::to_value(s.meta.origin)?;
                println!(
                    "{}	{}	{}	{}",
                    s.meta.name,
                    origin.as_str().unwrap_or_default(),
                    s.meta.version.as_deref().unwrap_or("-"),
                    s.meta.description
                );
            }
            for p in &set.problems {
                println!("INVALID	{}	{}", p.dir.display(), p.error);
            }
            if !a.skills.is_empty() {
                println!("(library skills enabled: {})", a.skills.join(", "));
            }
        }
        SkillCmd::Show { agent, name } => {
            let a = app.agent_named(agent)?;
            let set = reg.for_agent(&a.id.to_string(), &a.role, &a.skills);
            let s = set
                .get(name)
                .ok_or_else(|| anyhow!("{agent} has no skill {name:?}"))?;
            println!("{}", s.instructions());
        }
        SkillCmd::Library => {
            let set = reg.library();
            if set.is_empty() {
                println!("(the library is empty: agenium skill import <folder|zip> --library)");
            }
            let agents = app.agents()?;
            for s in &set.skills {
                let users: Vec<&str> = agents
                    .iter()
                    .filter(|a| a.skills.contains(&s.meta.name))
                    .map(|a| a.name.as_str())
                    .collect();
                println!(
                    "{}\t{}\t{}\t{}",
                    s.meta.name,
                    s.meta.version.as_deref().unwrap_or("-"),
                    s.meta.description,
                    if users.is_empty() {
                        "-".to_owned()
                    } else {
                        users.join(",")
                    }
                );
            }
            for p in &set.problems {
                println!("INVALID\t{}\t{}", p.dir.display(), p.error);
            }
        }
        SkillCmd::Import {
            source,
            agent,
            role,
            library: _,
            replace,
        } => {
            let owner = skill_owner(&app, agent, role)?;
            let tmp;
            let src = if source.starts_with("http://") || source.starts_with("https://") {
                tmp = std::env::temp_dir().join(format!("agenium-skill-{}", std::process::id()));
                std::fs::create_dir_all(&tmp)?;
                let r = download_zip(source, &tmp).await;
                if r.is_err() {
                    let _ = std::fs::remove_dir_all(&tmp);
                }
                r?
            } else {
                PathBuf::from(source)
            };
            let r = reg.import(&src, &owner, *replace);
            if source.starts_with("https://") {
                let _ = std::fs::remove_dir_all(src.parent().unwrap_or(&src));
            }
            let m = r?;
            println!("imported {} into {}", m.name, m.dir.display());
        }
        SkillCmd::Remove {
            name,
            agent,
            role,
            library: _,
            force,
        } => {
            match skill_owner(&app, agent, role)? {
                agenium_skills::Owner::Library => app.remove_library_skill(name, *force)?,
                owner => reg.remove(&owner, name)?,
            }
            println!("removed {name}");
        }
        SkillCmd::Validate { .. } => unreachable!(),
    }
    Ok(())
}
