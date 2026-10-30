//! Reflection on a proven lesson (ADR-146): once a lesson has held in two contexts, the agent's own model is asked —
//! beside the runs, once per lesson — whether to write it into a skill it has (its own copy), make a new skill of it,
//! or leave it as a memory. Skill writing used to wait for the agent to think of it in the middle of other work (1
//! run in 6 did); this asks at the moment the lesson is proven. What it proposes goes through every gate a proposal
//! made in a run goes through (`skill_propose`): the lesson's two contexts, the shape, one subject, the judge's
//! contradiction read. Nothing is written that a run could not have written.

use agenium_core::forms::{ProposeNewSkill, ProposeSkillUpdate};
use agenium_core::*;
use agenium_llm::{chat_with_retry, ChatRequest, Message, StopReason, SystemBlock, Usage};
use agenium_memory::pipeline::json_object;
use agenium_store::{AgentStore, Fact, FactKind};
use serde_json::{json, Value};

use super::Runtime;

/// The lessons reflected on (agent meta: a JSON list of fact ids).
const REFLECTED: &str = "reflected_lessons";
/// Lessons reflected on after one episode at most (the rest wait for the next).
const PER_EPISODE: usize = 2;

const DECIDE: &str = "You keep your own skills as an Agenium agent: short procedures you follow in your work. One of your \
lessons has now held in two different tickets, so it is proven. Decide what to do with it:\n\
- \"update\": a skill you have is about this situation (its description says when it applies) and the lesson is a rule, \
step or check it lacks: write the lesson into your own copy of that skill.\n\
- \"new\": the lesson is a procedure you will follow again (steps for a kind of work) and no skill of yours is about it: \
make a new skill.\n\
- \"none\": the lesson is a single fact, the skill already says it, or it is too narrow to guide other work.\n\
Prefer improving a skill you have over a new one. Answer with JSON only: {\"action\": \"update|new|none\", \"skill\": \
\"<the skill to update>\", \"reason\": \"...\"}";

const UPDATE: &str = "Write the lesson into your own copy of the skill below. Give the complete new body (everything \
after the frontmatter): keep everything that is still right, word for word; add the lesson where it belongs (a step, a \
rule, or a check in the proof), in the skill's own style; change nothing else. Answer with JSON only: {\"content\": \
\"...\", \"reason\": \"...\"}";

const NEW: &str = "Make a new skill of the lesson: a short procedure you will follow in future work. Give a name (1-64 \
characters of a-z, 0-9 and single hyphens, not the name of a skill you have), a one-line description of what it does \
and when to use it (\"... Use when ...\"), and a body with the sections ## Job, ## When to use, ## Steps, ## Tools and \
## Proof, each filled, the lesson's reason kept. Answer with JSON only: {\"name\": \"...\", \"description\": \"...\", \
\"body\": \"...\", \"reason\": \"...\"}";

impl Runtime {
    /// Reflects on the lessons of `agent` that are proven (held in two contexts) and not reflected on yet; returns what
    /// was done with each, for the run's log. Without a model for side calls, nothing.
    pub(super) async fn reflect_on_lessons(
        &self,
        agent: &agenium_store::core::agents::Agent,
        mem: &AgentStore,
    ) -> Vec<Value> {
        let mut done: Vec<i64> = mem
            .meta(REFLECTED)
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        let proven: Vec<Fact> = mem
            .live(2000)
            .unwrap_or_default()
            .into_iter()
            .filter(|f| {
                f.kind == FactKind::Lesson
                    && !f.untrusted
                    && f.validated.len() >= 2
                    && !done.contains(&f.id)
            })
            .take(PER_EPISODE)
            .collect();
        if proven.is_empty() {
            return vec![];
        }
        let Some((provider, model)) = self.side_model(agent) else {
            return vec![];
        };
        let mut out = Vec::new();
        for lesson in proven {
            let mut usage = Usage::default();
            let r = self
                .reflect_one(agent, &lesson, provider.as_ref(), &model, &mut usage)
                .await;
            if let Err(e) = self.record_side_call(
                agent.id,
                provider.id(),
                &model,
                &usage,
                json!({ "kind": "reflection", "lesson": lesson.id }),
                r.clone().unwrap_or_else(|e| json!({ "error": e })),
            ) {
                tracing::warn!(error = %e, "the reflection's usage was not recorded");
            }
            match r {
                Ok(v) => {
                    done.push(lesson.id);
                    out.push(v);
                }
                // The model could not be reached or answered nothing usable: the lesson is asked about again later.
                Err(e) => {
                    tracing::warn!(lesson = lesson.id, error = %e, "reflection on a lesson failed");
                    out.push(json!({ "lesson": lesson.id, "error": e }));
                }
            }
        }
        if let Err(e) = mem.set_meta(REFLECTED, &serde_json::to_string(&done).unwrap_or_default()) {
            tracing::warn!(error = %e, "the reflected lessons were not saved");
        }
        out
    }

    async fn ask_json(
        &self,
        provider: &dyn agenium_llm::Provider,
        model: &str,
        rules: &str,
        text: String,
        usage: &mut Usage,
    ) -> Result<Value, String> {
        let req = ChatRequest {
            model: model.to_owned(),
            system: vec![SystemBlock {
                text: rules.into(),
                cache: false,
            }],
            messages: vec![Message::user_text(text)],
            tools: vec![],
            effort: Effort::LOW,
            max_tokens: 6000,
            cache_key: None,
            long_cache: false,
            tools_off: false,
        };
        let r = chat_with_retry(provider, &req, &self.retry, &|_| {}, |_, _, _| {})
            .await
            .map_err(|e| e.to_string())?;
        usage.add(&r.usage);
        if r.stop == StopReason::MaxTokens {
            return Err("the answer was cut off".into());
        }
        let text = r.message.text();
        json_object(&text).ok_or_else(|| {
            format!(
                "the answer is not JSON: {}",
                text.chars().take(200).collect::<String>()
            )
        })
    }

    /// One lesson: decide, then write the skill through every gate. `Err` only when the model gave nothing usable.
    async fn reflect_one(
        &self,
        agent: &agenium_store::core::agents::Agent,
        lesson: &Fact,
        provider: &dyn agenium_llm::Provider,
        model: &str,
        usage: &mut Usage,
    ) -> Result<Value, String> {
        let set = self.skills(agent);
        let index: String = set
            .skills
            .iter()
            .map(|s| format!("- {}: {}\n", s.meta.name, s.meta.description))
            .collect();
        let about = format!(
            "## Lesson #{} (held in: {})\n{}\nWhen it applies: {}\n\n## Your skills\n{}",
            lesson.id,
            lesson.validated.join(", "),
            lesson.text,
            lesson.cue.as_deref().unwrap_or("-"),
            if index.is_empty() {
                "(none)\n".into()
            } else {
                index
            }
        );
        let d = self
            .ask_json(provider, model, DECIDE, about.clone(), usage)
            .await?;
        let action = d["action"].as_str().unwrap_or("none");
        let reason = d["reason"].as_str().unwrap_or("").to_owned();
        let base = json!({ "lesson": lesson.id, "action": action, "reason": reason });
        let app = &self.app;
        let result = match action {
            "update" => {
                let name = d["skill"].as_str().unwrap_or("").trim().to_owned();
                let Some(skill) = set.get(&name) else {
                    return Ok(
                        json!({ "lesson": lesson.id, "action": "update", "skill": name,
                        "result": "no such skill: nothing written" }),
                    );
                };
                let w = self
                    .ask_json(
                        provider,
                        model,
                        UPDATE,
                        format!(
                            "{about}\n## The skill {name} (its body now)\n{}",
                            skill.body
                        ),
                        usage,
                    )
                    .await?;
                let content = w["content"].as_str().unwrap_or("").to_owned();
                if content.trim().is_empty() {
                    return Err("no content for the skill".into());
                }
                let u = ProposeSkillUpdate {
                    skill: name.clone(),
                    patch: String::new(),
                    content: Some(content),
                    reason: w["reason"].as_str().map_or(reason.clone(), str::to_owned),
                    lesson: lesson.id,
                };
                json!({ "skill": name, "result": gate(app.propose_skill_update(agent.id, u, None).await) })
            }
            "new" => {
                let w = self.ask_json(provider, model, NEW, about, usage).await?;
                let u = ProposeNewSkill {
                    name: w["name"].as_str().unwrap_or("").trim().to_owned(),
                    description: w["description"].as_str().unwrap_or("").trim().to_owned(),
                    body: w["body"].as_str().unwrap_or("").to_owned(),
                    reason: w["reason"].as_str().map_or(reason.clone(), str::to_owned),
                    lesson: lesson.id,
                };
                let name = u.name.clone();
                json!({ "skill": name, "result": gate(app.propose_new_skill(agent.id, u).await) })
            }
            _ => json!({ "result": "kept as a memory" }),
        };
        let mut v = base;
        if let (Some(o), Some(r)) = (v.as_object_mut(), result.as_object()) {
            o.extend(r.clone());
        }
        Ok(v)
    }
}

/// What the gates said of a proposal, for the log.
fn gate(r: Result<Value, agenium_tools::ToolError>) -> Value {
    match r {
        Ok(v) => json!({ "accepted": v }),
        Err(e) => json!({ "refused": format!("{}: {}", e.code, e.message) }),
    }
}
