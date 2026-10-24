//! Requirement spec and plan (decision 3h): Discovery -> RequirementSpec (user approves, locked v1) -> Plan
//! (milestones + items tracing to requirements, user approves) -> tickets. Every ticket references a plan item.

use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::graph::find_cycle;
use crate::ticket::{is_ac_id, is_plan_item_id, AcceptanceCriterion};
use crate::validate::{Checker, FormErrors, Validate};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    /// Requirement id, e.g. REQ-1.
    #[schemars(pattern(r"^REQ-[0-9]+$"))]
    pub id: String,
    pub text: String,
    #[schemars(length(min = 1))]
    pub acceptance: Vec<AcceptanceCriterion>,
}

/// Requirement spec proposed by the Lead after Discovery; waits for user approval, then is locked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequirementSpec {
    #[schemars(length(min = 1))]
    pub goals: Vec<String>,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub out_of_scope: Vec<String>,
    #[schemars(length(min = 1))]
    pub requirements: Vec<Requirement>,
    #[serde(default)]
    pub constraints: Vec<String>,
}

pub fn is_requirement_id(s: &str) -> bool {
    s.strip_prefix("REQ-")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn check_acceptance(c: &mut Checker, field: &str, acs: &[AcceptanceCriterion]) {
    c.items(field, acs, 1, 100);
    let mut seen = HashSet::new();
    for (i, a) in acs.iter().enumerate() {
        c.check(
            is_ac_id(&a.id),
            format!("{field}[{i}].id"),
            "must look like AC-1",
        );
        c.check(
            seen.insert(a.id.as_str()),
            format!("{field}[{i}].id"),
            "duplicate id",
        );
        c.non_blank(&format!("{field}[{i}].text"), &a.text);
    }
}

impl Validate for RequirementSpec {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.items("goals", &self.goals, 1, 50);
        c.items("requirements", &self.requirements, 1, 500);
        let mut ids = HashSet::new();
        for (i, r) in self.requirements.iter().enumerate() {
            c.check(
                is_requirement_id(&r.id),
                format!("requirements[{i}].id"),
                "must look like REQ-1",
            );
            c.check(
                ids.insert(r.id.as_str()),
                format!("requirements[{i}].id"),
                "duplicate id",
            );
            c.non_blank(&format!("requirements[{i}].text"), &r.text);
            check_acceptance(
                &mut c,
                &format!("requirements[{i}].acceptance"),
                &r.acceptance,
            );
        }
        c.finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Milestone {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Estimate {
    S,
    M,
    L,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanItem {
    /// Plan item id, e.g. PI-1.
    #[schemars(pattern(r"^PI-[0-9]+$"))]
    pub id: String,
    pub milestone: String,
    pub title: String,
    /// Requirement ids this item implements, e.g. REQ-2.
    #[schemars(length(min = 1))]
    pub trace_to: Vec<String>,
    #[schemars(length(min = 1))]
    pub acceptance: Vec<AcceptanceCriterion>,
    /// Role expected to do the work.
    pub role: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub estimate: Estimate,
    /// Files/areas the item touches (glob patterns).
    #[serde(default)]
    pub scope_paths: Vec<String>,
}

/// A plan built from the locked spec; waits for user approval (Solo: self-approved, the user is informed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TicketPlan {
    #[schemars(range(min = 1))]
    pub spec_version: u32,
    #[schemars(length(min = 1))]
    pub milestones: Vec<Milestone>,
    #[schemars(length(min = 1))]
    pub items: Vec<PlanItem>,
}

/// Plan v+1 with a reason (replaces split_ticket). Scope changes need user approval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanRevision {
    /// The plan version this revision is based on.
    pub base_version: u32,
    #[schemars(length(min = 5))]
    pub reason: String,
    #[schemars(length(min = 1))]
    pub milestones: Vec<Milestone>,
    #[schemars(length(min = 1))]
    pub items: Vec<PlanItem>,
}

/// The planner's draft of plan v1 (ADR-59): the Lead reviews it, adjusts it and calls create_plan; the rationale
/// stays visible to the Lead for later revisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanDraft {
    /// The locked spec version the draft is for.
    #[schemars(range(min = 1))]
    pub spec_version: u32,
    #[schemars(length(min = 1))]
    pub milestones: Vec<Milestone>,
    #[schemars(length(min = 1))]
    pub items: Vec<PlanItem>,
    /// Why the work is split this way: order, dependencies, scopes, risks, what to watch when revising.
    #[schemars(length(min = 20))]
    pub rationale: String,
}

impl PlanDraft {
    pub fn plan(&self) -> TicketPlan {
        TicketPlan {
            spec_version: self.spec_version,
            milestones: self.milestones.clone(),
            items: self.items.clone(),
        }
    }
}

impl Validate for PlanDraft {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.check(self.spec_version >= 1, "spec_version", "must be >= 1");
        check_plan(&mut c, &self.milestones, &self.items);
        c.len("rationale", self.rationale.trim(), 20, 8000);
        c.finish()
    }
}

fn check_plan(c: &mut Checker, milestones: &[Milestone], items: &[PlanItem]) {
    c.items("milestones", milestones, 1, 100);
    c.items("items", items, 1, 1000);
    let ms: HashSet<&str> = milestones.iter().map(|m| m.id.as_str()).collect();
    c.check(
        ms.len() == milestones.len(),
        "milestones",
        "duplicate milestone id",
    );
    let mut ids = HashSet::new();
    for (i, it) in items.iter().enumerate() {
        let f = |s: &str| format!("items[{i}].{s}");
        c.check(is_plan_item_id(&it.id), f("id"), "must look like PI-1");
        c.check(ids.insert(it.id.as_str()), f("id"), "duplicate id");
        c.check(
            ms.contains(it.milestone.as_str()),
            f("milestone"),
            "unknown milestone",
        );
        c.non_blank(&f("title"), &it.title);
        c.items(&f("trace_to"), &it.trace_to, 1, 50);
        for t in &it.trace_to {
            c.check(
                is_requirement_id(t),
                f("trace_to"),
                format!("{t:?} must look like REQ-1"),
            );
        }
        check_acceptance(c, &f("acceptance"), &it.acceptance);
    }
    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        for d in &it.depends_on {
            c.check(
                ids.contains(d.as_str()),
                format!("items[{i}].depends_on"),
                format!("unknown plan item {d}"),
            );
        }
        let deps: Vec<&str> = it.depends_on.iter().map(String::as_str).collect();
        if let Some(cycle) = find_cycle(&graph, &it.id.as_str(), &deps) {
            c.fail(
                format!("items[{i}].depends_on"),
                format!("dependency cycle: {}", cycle.join(" -> ")),
            );
        }
        graph.insert(it.id.as_str(), deps);
    }
}

impl Validate for TicketPlan {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.check(self.spec_version >= 1, "spec_version", "must be >= 1");
        check_plan(&mut c, &self.milestones, &self.items);
        c.finish()
    }
}

impl Validate for PlanRevision {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.len("reason", self.reason.trim(), 5, 2000);
        check_plan(&mut c, &self.milestones, &self.items);
        c.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_plan_example_validates() {
        let p: TicketPlan = serde_json::from_value(serde_json::json!({ "spec_version": 1,
          "milestones": [{ "id": "M1", "title": "Google login" }],
          "items": [
            { "id": "PI-1", "milestone": "M1", "title": "OAuth backend", "trace_to": ["REQ-2"],
              "acceptance": [{ "id": "AC-1", "text": "Login via Google returns a session" }],
              "role": "backend", "depends_on": [], "estimate": "M" },
            { "id": "PI-2", "milestone": "M1", "title": "Login button", "trace_to": ["REQ-2"],
              "acceptance": [{ "id": "AC-1", "text": "Button starts OAuth flow" }],
              "role": "frontend", "depends_on": ["PI-1"], "estimate": "S" } ] }))
        .unwrap();
        p.validate().unwrap();
    }

    #[test]
    fn plan_rejects_cycles_and_unknown_refs() {
        let item = |id: &str, deps: &[&str]| PlanItem {
            id: id.into(),
            milestone: "M1".into(),
            title: "t".into(),
            trace_to: vec!["REQ-1".into()],
            acceptance: vec![AcceptanceCriterion {
                id: "AC-1".into(),
                text: "x".into(),
            }],
            role: "worker".into(),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            estimate: Estimate::S,
            scope_paths: vec![],
        };
        let p = TicketPlan {
            spec_version: 1,
            milestones: vec![Milestone {
                id: "M1".into(),
                title: "m".into(),
            }],
            items: vec![
                item("PI-1", &["PI-2"]),
                item("PI-2", &["PI-1"]),
                item("PI-3", &["PI-9"]),
            ],
        };
        let e = p.validate().unwrap_err().to_string();
        assert!(
            e.contains("dependency cycle") && e.contains("unknown plan item PI-9"),
            "{e}"
        );
    }
}
