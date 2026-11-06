//! Acceptance in the chat (ADR-158): when a review closes a ticket whose report has screenshots, the Lead's chat with
//! the user says so with them, once, without the user asking. The chat stores a line per image (`SHOT_LINE`), which
//! the desktop shows as the image; the Lead's model reads it as plain text in later turns.

use super::*;

/// How a screenshot is named in a chat message: `Screenshot of <KEY>: <file name>`.
pub const SHOT_LINE: &str = "Screenshot of ";
/// At most this many images per accepted ticket in the chat (the rest stay on the ticket).
const MAX_SHOWN: usize = 4;

/// `(ticket key, file name)` when `line` names a screenshot as `SHOT_LINE` writes it.
pub fn shot_of(line: &str) -> Option<(&str, &str)> {
    let (key, name) = line.trim().strip_prefix(SHOT_LINE)?.split_once(": ")?;
    let ok_key = key
        .split_once('-')
        .is_some_and(|(p, n)| !p.is_empty() && n.parse::<u32>().is_ok());
    (ok_key && !name.trim().is_empty() && !name.contains(['/', '\\'])).then(|| (key, name.trim()))
}

impl Runtime {
    /// After the review of `key`: when it closed the ticket and the report kept screenshots, they go into the chat.
    pub(super) fn show_acceptance(&self, project: ProjectId, key: &TicketKey) -> Result<()> {
        let t = self.app.ticket(project, key)?;
        if t.status != TicketStatus::Closed {
            return Ok(());
        }
        let p = self.app.project(project)?;
        let dir = crate::evidence::dir(std::path::Path::new(&p.path), key.as_str());
        let mut shots: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| {
                crate::evidence::read(std::path::Path::new(&p.path), key.as_str(), n).is_some()
            })
            .collect();
        if shots.is_empty() {
            return Ok(());
        }
        shots.sort();
        let Some(lead) = self.app.team_of_project(project)?.and_then(|(t, _)| t.lead) else {
            return Ok(());
        };
        let mem = self.agent_memory(lead)?;
        let mark = format!("acceptance-shown:{}", t.id);
        if mem.meta(&mark)?.is_some() {
            return Ok(());
        }
        let mut text = format!("{key} is done and accepted: {}.", t.title);
        if shots.len() > MAX_SHOWN {
            text.push_str(&format!(
                " {} screenshots are on the ticket; the first {MAX_SHOWN}:",
                shots.len()
            ));
        }
        text.push('\n');
        for s in shots.iter().take(MAX_SHOWN) {
            text.push_str(&format!("\n{SHOT_LINE}{key}: {s}"));
        }
        let v = serde_json::to_value(agenium_llm::Message::assistant_text(text))
            .map_err(|e| OrchError::Invalid(e.to_string()))?;
        mem.append_message(&learn::chat_session(project), &v)?;
        mem.set_meta(&mark, "1")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::shot_of;

    #[test]
    fn a_screenshot_line_is_read_back_and_nothing_else_is() {
        assert_eq!(
            shot_of("Screenshot of HB-6: page.png"),
            Some(("HB-6", "page.png"))
        );
        assert_eq!(
            shot_of("  Screenshot of AGN-12: a b.png "),
            Some(("AGN-12", "a b.png"))
        );
        for no in [
            "Screenshot of HB: x.png",
            "Screenshot of HB-6: ../x.png",
            "Screenshot of HB-6:",
            "HB-6: x.png",
        ] {
            assert_eq!(shot_of(no), None, "{no}");
        }
    }
}
