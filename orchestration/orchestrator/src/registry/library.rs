//! The system skill library and MCP servers removed with their bindings.

use super::*;

impl App {
    /// Imports a skill (folder, `.zip`) into the system library.
    pub fn import_library_skill(
        &self,
        src: &Path,
        replace: bool,
    ) -> Result<agenium_skills::SkillMeta> {
        self.skill_registry()
            .import(src, &agenium_skills::Owner::Library, replace)
            .map_err(io)
    }

    /// The skills a folder holds, for importing many at once: each with whether the library has one of that name.
    pub fn scan_skill_folder(
        &self,
        dir: &Path,
    ) -> Result<Vec<(agenium_skills::scan::Found, bool)>> {
        if !dir.is_dir() {
            return Err(OrchError::Invalid(format!(
                "{} is not a folder",
                dir.display()
            )));
        }
        let have: Vec<String> = self
            .skill_registry()
            .library()
            .skills
            .into_iter()
            .map(|s| s.meta.name)
            .collect();
        Ok(agenium_skills::scan::scan(dir)
            .into_iter()
            .map(|f| {
                let exists = have.contains(&f.name);
                (f, exists)
            })
            .collect())
    }

    /// Imports several skill folders: each succeeds or fails on its own (name, or why not).
    pub fn import_library_skills(
        &self,
        dirs: &[std::path::PathBuf],
        replace: bool,
    ) -> Vec<std::result::Result<String, String>> {
        dirs.iter()
            .map(|d| {
                self.import_library_skill(d, replace)
                    .map(|m| m.name)
                    .map_err(|e| format!("{}: {e}", d.display()))
            })
            .collect()
    }

    /// Writes a library skill's SKILL.md from text (create a new one or edit in place) together with its files: `files`
    /// added or changed, `removed` deleted, the rest kept; the whole folder is validated first. The frontmatter `name`
    /// must equal `name`.
    pub fn save_library_skill(
        &self,
        name: &str,
        content: &str,
        create: bool,
        files: &[(String, Vec<u8>)],
        removed: &[String],
    ) -> Result<agenium_skills::SkillMeta> {
        self.skill_registry()
            .write_with_files(
                &agenium_skills::Owner::Library,
                name,
                content,
                create,
                files,
                removed,
            )
            .map_err(io)
    }

    /// Removes a library skill. Agents that enable it lose it when `force`; otherwise refused while in use.
    pub fn remove_library_skill(&self, name: &str, force: bool) -> Result<()> {
        let users: Vec<agents::Agent> = self
            .agents()?
            .into_iter()
            .filter(|a| a.skills.iter().any(|s| s == name))
            .collect();
        if !users.is_empty() && !force {
            return Err(OrchError::Conflict(format!(
                "agents use skill {name}: {}",
                users
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        self.skill_registry()
            .remove(&agenium_skills::Owner::Library, name)
            .map_err(io)?;
        for a in users {
            let rest: Vec<String> = a.skills.into_iter().filter(|s| s != name).collect();
            self.set_agent_skills(a.id, &rest)?;
        }
        Ok(())
    }

    /// Removes an MCP server; with `force` the agents that enable it lose it first.
    pub fn remove_mcp_server_force(&self, id: &str) -> Result<()> {
        for a in self.agents()? {
            let b = agenium_mcp::parse_bindings(&a.mcp).map_err(io)?;
            if b.iter().any(|x| x.server == id) {
                let rest: Vec<_> = b.into_iter().filter(|x| x.server != id).collect();
                self.set_agent_mcp(a.id, &rest)?;
            }
        }
        self.remove_mcp_server(id)
    }

    // ------------------------------------------------------------ providers
}
