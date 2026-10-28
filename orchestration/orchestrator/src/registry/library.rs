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

    /// Writes a library skill's SKILL.md from text (create a new one or edit in place), validating it; its other
    /// files are kept. The frontmatter `name` must equal `name`.
    pub fn save_library_skill(
        &self,
        name: &str,
        content: &str,
        create: bool,
    ) -> Result<agenium_skills::SkillMeta> {
        self.skill_registry()
            .write(&agenium_skills::Owner::Library, name, content, create)
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
