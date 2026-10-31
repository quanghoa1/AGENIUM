//! The user's last choices (ADR-152): the model picked in each place a model is picked, kept in the root so a form or
//! a helper chat opens with it after a restart, instead of empty.

use agenium_store::core::preferences;
use serde_json::Value;

use crate::app::App;
use crate::{OrchError, Result};

/// The keys a choice is kept under (anything else is refused: the table is not a general store).
pub const KEYS: [&str; 4] = [
    "model.agent",
    "model.team",
    "model.skill_writer",
    "model.designer",
];

/// A kept value at most (bytes).
const MAX: usize = 4096;

impl App {
    pub fn preference(&self, key: &str) -> Result<Option<Value>> {
        known(key)?;
        Ok(self.store().read(|c| preferences::get(c, key))?)
    }

    pub fn set_preference(&self, key: &str, value: &Value) -> Result<()> {
        known(key)?;
        if value.to_string().len() > MAX {
            return Err(OrchError::Invalid(format!(
                "the value of {key} is too large"
            )));
        }
        self.store().commit(None, "user", |w| {
            preferences::set(w.tx, key, value, &agenium_core::now_iso()).map_err(OrchError::from)
        })?;
        Ok(())
    }
}

fn known(key: &str) -> Result<()> {
    if KEYS.contains(&key) {
        Ok(())
    } else {
        Err(OrchError::Invalid(format!("{key:?} is not a kept choice")))
    }
}
