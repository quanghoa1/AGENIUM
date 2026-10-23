//! Identifiers. Internal ids are ULIDs (sortable, collision free); agents carry an `agent_` prefix so an id in a
//! prompt or log line is self-describing. Ticket keys (`AGN-12`) are the human/agent facing handle, unique per project.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

/// Error returned when a string is not a valid id or key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind}: {value:?}")]
pub struct IdParseError {
    pub kind: &'static str,
    pub value: String,
}

macro_rules! ulid_id {
    ($(#[$meta:meta])* $name:ident, $prefix:expr) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Ulid);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self(Ulid::new())
            }

            pub fn from_ulid(u: Ulid) -> Self {
                Self(u)
            }

            pub fn ulid(&self) -> Ulid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", Self::PREFIX, self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let err = || IdParseError { kind: stringify!($name), value: s.to_owned() };
                let rest = s.strip_prefix(Self::PREFIX).ok_or_else(err)?;
                Ulid::from_string(rest).map(Self).map_err(|_| err())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = Cow::<str>::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }
            fn inline_schema() -> bool {
                true
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "string",
                    "pattern": concat!("^", $prefix, "[0-9A-HJKMNP-TV-Z]{26}$")
                })
            }
        }
    };
}

ulid_id!(
    /// A project (a git repository with `agenium.toml`).
    ProjectId, ""
);
ulid_id!(
    /// Internal ticket id; agents and users use [`TicketKey`].
    TicketId, ""
);
ulid_id!(
    /// A long-lived agent of the registry.
    AgentId, "agent_"
);
ulid_id!(
    /// A team (exactly one per agent).
    TeamId, ""
);
ulid_id!(
    /// A typed comment on a ticket.
    CommentId, ""
);
ulid_id!(
    /// One execution of an agent loop.
    RunId, ""
);
ulid_id!(
    /// Idempotency key of an intent (a validated tool call or a system action).
    IntentId, ""
);

/// Human/agent facing ticket handle, `<PREFIX>-<n>` (e.g. `AGN-12`), unique within a project.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TicketKey(String);

impl TicketKey {
    pub fn new(prefix: &str, n: u64) -> Result<Self, IdParseError> {
        format!("{prefix}-{n}").parse()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn prefix(&self) -> &str {
        self.0.split_once('-').map(|(p, _)| p).unwrap_or_default()
    }

    pub fn number(&self) -> u64 {
        self.0
            .split_once('-')
            .and_then(|(_, n)| n.parse().ok())
            .unwrap_or_default()
    }

    /// `true` when `prefix` is a valid key prefix (1-10 ASCII uppercase letters).
    pub fn valid_prefix(prefix: &str) -> bool {
        (1..=10).contains(&prefix.len()) && prefix.bytes().all(|b| b.is_ascii_uppercase())
    }
}

impl FromStr for TicketKey {
    type Err = IdParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || IdParseError {
            kind: "TicketKey",
            value: s.to_owned(),
        };
        let (prefix, n) = s.split_once('-').ok_or_else(err)?;
        let digits_ok = !n.is_empty()
            && n.len() <= 12
            && n.bytes().all(|b| b.is_ascii_digit())
            && !n.starts_with('0');
        if !Self::valid_prefix(prefix) || !digits_ok {
            return Err(err());
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for TicketKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for TicketKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for TicketKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TicketKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = Cow::<str>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for TicketKey {
    fn schema_name() -> Cow<'static, str> {
        "TicketKey".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^[A-Z]{1,10}-[1-9][0-9]*$",
            "description": "Ticket key, e.g. AGN-11"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_with_prefix() {
        let a = AgentId::new();
        let s = a.to_string();
        assert!(s.starts_with("agent_") && s.len() == 32);
        assert_eq!(s.parse::<AgentId>().unwrap(), a);
        assert!(
            s[6..].parse::<AgentId>().is_err(),
            "missing prefix must be rejected"
        );
        let t = TicketId::new();
        assert_eq!(serde_json::to_string(&t).unwrap(), format!("\"{t}\""));
        assert_eq!(
            serde_json::from_str::<TicketId>(&format!("\"{t}\"")).unwrap(),
            t
        );
    }

    #[test]
    fn ticket_keys_validate() {
        let k: TicketKey = "AGN-12".parse().unwrap();
        assert_eq!((k.prefix(), k.number()), ("AGN", 12));
        for bad in [
            "agn-1",
            "AGN-",
            "AGN-0",
            "AGN-01",
            "-1",
            "AGN1",
            "ABCDEFGHIJK-1",
            "AGN-1a",
        ] {
            assert!(bad.parse::<TicketKey>().is_err(), "{bad}");
        }
        assert_eq!(TicketKey::new("X", 7).unwrap().as_str(), "X-7");
    }
}
