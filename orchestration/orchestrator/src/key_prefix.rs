//! A project's ticket key prefix (the `UIDE` of `UIDE-12`): the user's own, or one made from the project's name, unique
//! among the projects (a prefix two projects share made their tickets' keys ambiguous).

use agenium_core::TicketKey;

use crate::app::App;
use crate::{OrchError, Result};

/// The letters a prefix is made from: a several-word name gives its initials, a one-word name its first four letters
/// (accents dropped; a name without ASCII letters gives `PRJ`).
fn base(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| {
            w.chars()
                .map(plain)
                .filter(char::is_ascii_alphabetic)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_uppercase())
        .collect();
    match words.as_slice() {
        [] => "PRJ".into(),
        [one] => one.chars().take(4).collect(),
        many => many
            .iter()
            .filter_map(|w| w.chars().next())
            .take(10)
            .collect(),
    }
}

/// A Latin letter without its accent (Vietnamese and the common Western ones); any other character as it is.
fn plain(c: char) -> char {
    const FROM: &str = "àáạảãâầấậẩẫăằắặẳẵèéẹẻẽêềếệểễìíịỉĩòóọỏõôồốộổỗơờớợởỡùúụủũưừứựửữỳýỵỷỹđçñäöüÀÁẠẢÃÂẦẤẬẨẪĂẰẮẶẲẴÈÉẸẺẼÊỀẾỆỂỄÌÍỊỈĨÒÓỌỎÕÔỒỐỘỔỖƠỜỚỢỞỠÙÚỤỦŨƯỪỨỰỬỮỲÝỴỶỸĐÇÑÄÖÜ";
    const TO: &str = "aaaaaaaaaaaaaaaaaeeeeeeeeeeeiiiiiooooooooooooooooouuuuuuuuuuuyyyyydcnaouAAAAAAAAAAAAAAAAAEEEEEEEEEEEIIIIIOOOOOOOOOOOOOOOOOUUUUUUUUUUUYYYYYDCNAOU";
    FROM.chars()
        .position(|f| f == c)
        .and_then(|i| TO.chars().nth(i))
        .unwrap_or(c)
}

impl App {
    /// A free prefix made from `name`: its base, then longer runs of its letters, then the base with a letter added.
    pub fn suggest_key_prefix(&self, name: &str) -> Result<String> {
        let taken: Vec<String> = self.projects()?.into_iter().map(|p| p.key_prefix).collect();
        let free = |p: &str| !taken.iter().any(|t| t == p);
        let base = base(name);
        let letters: String = name
            .chars()
            .map(plain)
            .filter(char::is_ascii_alphabetic)
            .map(|c| c.to_ascii_uppercase())
            .collect();
        let longer = (base.len() + 1..=letters.len().min(10)).map(|n| letters[..n].to_owned());
        let lettered = ('A'..='Z').map(|c| format!("{}{c}", &base[..base.len().min(9)]));
        std::iter::once(base.clone())
            .chain(longer)
            .chain(lettered)
            .find(|p| free(p))
            .ok_or_else(|| {
                OrchError::Conflict(format!("no free ticket prefix from {name:?}: give one"))
            })
    }

    /// The user's own prefix, checked: 1 to 10 capital letters, used by no other project.
    pub fn check_key_prefix(&self, prefix: &str) -> Result<()> {
        if !TicketKey::valid_prefix(prefix) {
            return Err(OrchError::Invalid(format!(
                "the ticket prefix {prefix:?} must be 1 to 10 capital letters (A-Z)"
            )));
        }
        if let Some(p) = self
            .projects()?
            .into_iter()
            .find(|p| p.key_prefix == prefix)
        {
            return Err(OrchError::Conflict(format!(
                "the ticket prefix {prefix} is the project {}'s: pick another",
                p.name
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::base;

    #[test]
    fn a_prefix_is_made_from_the_name() {
        assert_eq!(base("uidemo"), "UIDE");
        assert_eq!(base("test"), "TEST");
        assert_eq!(base("My shop app"), "MSA");
        assert_eq!(base("Dự án bán hàng"), "DABH");
        assert_eq!(base("x"), "X");
        assert_eq!(base("日本"), "PRJ");
    }
}
