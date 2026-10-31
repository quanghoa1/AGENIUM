//! `attribution`: the extraction alone, on a real model, over random cases (see the module above).

use std::sync::Arc;
use std::time::Instant;

use agenium_core::*;
use agenium_llm::Provider;
use agenium_memory::pipeline::{learn, Episode, PipelineModel};
use agenium_memory::{Embedder, HashEmbedder};
use agenium_store::{AgentStore, FactKind, NewFact};
use serde_json::{json, Value};

use super::ID;
use crate::TaskResult;

/// A lesson right elsewhere and wrong in this project, with what following it and not following it looks like.
struct Topic {
    wrong: &'static str,
    cue: &'static str,
    ac: &'static str,
    followed: &'static str,
    feedback: &'static str,
    right: &'static str,
}

const TOPICS: [Topic; 8] = [
    Topic {
        wrong: "Every team document in this project starts with the line 'Owner: platform-team'.",
        cue: "when writing a team document",
        ac: "deliverables/hello.txt names its owning team on its first line",
        followed: "wrote deliverables/hello.txt with the first line 'Owner: platform-team'",
        feedback: "AC-1 not met: the first line says 'Owner: platform-team'; this project's documents belong to the docs team ('Owner: docs-team').",
        right: "wrote deliverables/hello.txt with the first line 'Owner: docs-team', as README.md says",
    },
    Topic {
        wrong: "Run this project's tests with `npm test`.",
        cue: "when running the tests",
        ac: "the new date helper is covered by a passing test run",
        followed: "ran `npm test`, which stopped on the lockfile, and reported the tests as run",
        feedback: "AC-1 not met: the project uses pnpm; `npm test` fails on the pnpm lockfile. Run `pnpm test`.",
        right: "ran `pnpm test` (package.json names pnpm): 12 passed",
    },
    Topic {
        wrong: "Dates in this project's reports are written as MM/DD/YYYY.",
        cue: "when writing a date in a report",
        ac: "reports/weekly.md dates each of its entries",
        followed: "dated the entries 10/31/2026",
        feedback: "AC-1 not met: report dates are ISO 8601 (2026-10-31), not 10/31/2026.",
        right: "dated the entries 2026-10-31, as the other reports do",
    },
    Topic {
        wrong: "Application settings in this project live in config/settings.json.",
        cue: "when adding a setting",
        ac: "the retry limit can be configured",
        followed: "added retry_limit to config/settings.json",
        feedback: "AC-1 not met: settings are read from settings/app.toml; config/settings.json is never loaded.",
        right: "added retry_limit to settings/app.toml next to the other settings",
    },
    Topic {
        wrong: "Commit subjects in this project start with a type prefix, for example 'feat: add greeting'.",
        cue: "when writing a commit message",
        ac: "the change is committed with a subject naming the ticket",
        followed: "committed as 'feat: add greeting'",
        feedback: "AC-1 not met: commit subjects start with the ticket key ('AGN-7: Add greeting').",
        right: "committed as 'AGN-7: Add greeting'",
    },
    Topic {
        wrong: "Log messages in this project are lower case without a final period.",
        cue: "when writing a log message",
        ac: "failed logins are logged",
        followed: "logged 'login failed for user'",
        feedback: "AC-1 not met: log messages are full sentences, capitalised and ending with a period ('Login failed for user.').",
        right: "logged 'Login failed for user.'",
    },
    Topic {
        wrong: "HTTP handlers in this project answer errors with status 200 and an \"error\" field.",
        cue: "when an HTTP handler fails",
        ac: "a request for a missing order answers with an error",
        followed: "returned status 200 with {\"error\": \"not found\"}",
        feedback: "AC-1 not met: a missing order must answer 404; errors use their HTTP status here.",
        right: "returned 404 with the error body the other handlers use",
    },
    Topic {
        wrong: "Python scripts under tools/ are indented with tabs.",
        cue: "when writing a Python script under tools/",
        ac: "tools/cleanup.py passes the project's lint",
        followed: "indented tools/cleanup.py with tabs",
        feedback: "AC-1 not met: the lint (ruff, PEP 8) rejects tabs; indent with four spaces.",
        right: "indented tools/cleanup.py with four spaces",
    },
];

/// Lessons that have nothing to do with any topic or with the changelog.
const DISTRACTORS: [(&str, &str); 8] = [
    (
        "Database migrations in this project run before the integration tests.",
        "when running the integration tests",
    ),
    (
        "Images in the docs are stored as PNG under docs/img/.",
        "when adding an image to the docs",
    ),
    (
        "Feature flags in this project are named in kebab-case.",
        "when adding a feature flag",
    ),
    (
        "Background jobs here retry three times with exponential backoff.",
        "when writing a background job",
    ),
    (
        "The CI cache key includes the lockfile's hash.",
        "when changing the CI cache",
    ),
    (
        "Environment variables of this project are prefixed with APP_.",
        "when adding an environment variable",
    ),
    (
        "Public functions get a doc comment with an example.",
        "when adding a public function",
    ),
    (
        "Fixtures for the API tests live in tests/fixtures/.",
        "when adding an API test fixture",
    ),
];

/// Where the reopened work went wrong.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    /// It followed the agent's own wrong lesson.
    OwnFollowed,
    /// It followed the shared wrong lesson.
    SharedFollowed,
    /// The agent's own wrong lesson was shown but not followed; the work failed on something else.
    OwnNotFollowed,
    /// The shared wrong lesson was shown but not followed; the work failed on something else.
    SharedNotFollowed,
    /// No lesson shown is about what failed.
    Unrelated,
}

const KINDS: [Kind; 5] = [
    Kind::OwnFollowed,
    Kind::SharedFollowed,
    Kind::OwnNotFollowed,
    Kind::SharedNotFollowed,
    Kind::Unrelated,
];

/// A small seeded generator (splitmix64): the cases of a seed are the same on every machine.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
}

/// One random case: the lessons shown (own, then shared with their pool ids), the episode text and the right answer.
struct Case {
    kind: Kind,
    quoted: bool,
    topic: usize,
    own: Vec<(&'static str, &'static str)>,
    shared: Vec<(i64, &'static str)>,
    transcript: String,
}

fn case(rng: &mut Rng, n: usize) -> Case {
    let kind = KINDS[rng.below(KINDS.len())];
    let topic = rng.below(TOPICS.len());
    let t = &TOPICS[topic];
    // Different distractors on each side, so a text is never both the agent's and the pool's.
    let mut d: Vec<usize> = (0..DISTRACTORS.len()).collect();
    rng.shuffle(&mut d);
    let mut own: Vec<(&str, &str)> = d[..rng.below(3)].iter().map(|&i| DISTRACTORS[i]).collect();
    let mut shared_texts: Vec<&str> = d[4..4 + rng.below(3)]
        .iter()
        .map(|&i| DISTRACTORS[i].0)
        .collect();
    match kind {
        Kind::OwnFollowed | Kind::OwnNotFollowed => own.push((t.wrong, t.cue)),
        Kind::SharedFollowed | Kind::SharedNotFollowed => shared_texts.push(t.wrong),
        Kind::Unrelated => {}
    }
    if own.is_empty() && shared_texts.is_empty() {
        shared_texts.push(DISTRACTORS[d[7]].0);
    }
    rng.shuffle(&mut own);
    rng.shuffle(&mut shared_texts);
    // Pool ids among the agent's own (1, 2, ...): they collide often.
    let mut ids: Vec<i64> = (1..=5).collect();
    rng.shuffle(&mut ids);
    let shared: Vec<(i64, &str)> = shared_texts
        .into_iter()
        .zip(ids)
        .map(|(t, id)| (id, t))
        .collect();
    let quoted = rng.below(2) == 0;
    let followed = matches!(kind, Kind::OwnFollowed | Kind::SharedFollowed);
    let mut work = String::new();
    if followed {
        if quoted {
            work.push_str(&format!(
                "[Assistant] A memory shown to me says: \"{}\" I will follow it.\n",
                t.wrong
            ));
        } else {
            work.push_str("[Assistant] I know how this project does it; writing the change.\n");
        }
        work.push_str(&format!(
            "[tool call] write_file\n[agenium result] write_file: ok\n[Assistant] Done: I {}, and added the change to CHANGELOG.md.\n",
            t.followed
        ));
    } else {
        if matches!(kind, Kind::OwnNotFollowed | Kind::SharedNotFollowed) && quoted {
            work.push_str(&format!(
                "[Assistant] A memory says \"{}\", but the project's own files say otherwise here; I follow the project.\n",
                t.wrong
            ));
        }
        work.push_str(&format!(
            "[tool call] write_file\n[agenium result] write_file: ok\n[Assistant] Done: I {}.\n",
            t.right
        ));
    }
    work.push_str("[tool call] submit_report\n[agenium result] submit_report: report accepted\n");
    let feedback = if followed {
        t.feedback.to_owned()
    } else {
        "AC-2 not met: CHANGELOG.md does not list the change.".to_owned()
    };
    let transcript = format!(
        "Ticket AGN-{n}: {title}\nAcceptance criteria:\n- AC-1: {ac}\n- AC-2: CHANGELOG.md lists the change\n\n\
         ## Comments\n[Lead review] Reopen.\n- {feedback}\n\n## Your work conversation\n{work}",
        title = t.ac,
        ac = t.ac,
    );
    Case {
        kind,
        quoted,
        topic,
        own,
        shared,
        transcript,
    }
}

fn pipeline_model(model: &str) -> PipelineModel {
    PipelineModel {
        model: model.into(),
        effort: Effort::LOW,
        max_tokens: 8000,
        retry: agenium_llm::RetryPolicy::default(),
    }
}

/// Runs one case through the real learning pipeline on a fresh memory; returns its graded record.
async fn attribute(
    provider: Arc<dyn Provider>,
    model: String,
    c: Case,
    n: usize,
) -> anyhow::Result<Value> {
    let tmp = tempfile::tempdir()?;
    let e: Arc<dyn Embedder> = Arc::new(HashEmbedder::new(384));
    let store = AgentStore::open(
        &tmp.path().join("a.db"),
        384,
        e.model_id(),
        &tmp.path().join("b"),
    )?;
    let mut used = Vec::new();
    for (text, cue) in &c.own {
        let id = store.add(
            &NewFact {
                kind: FactKind::Lesson,
                cue: Some((*cue).into()),
                confidence: 0.7,
                ..NewFact::fact(*text, "ticket:AGN-1")
            },
            None,
        )?;
        used.push((id, (*text).to_owned()));
    }
    let run = format!("run-{n}");
    store.record_usage(
        &run,
        Some("AGN-9"),
        &used
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i as u32 + 1, agenium_store::Via::Hybrid))
            .collect::<Vec<_>>(),
    )?;
    let t = &TOPICS[c.topic];
    let culprit_own = used
        .iter()
        .find(|(_, x)| x == t.wrong && c.kind == Kind::OwnFollowed)
        .map(|(id, _)| *id);
    let culprit_shared = c
        .shared
        .iter()
        .find(|(_, x)| *x == t.wrong && c.kind == Kind::SharedFollowed)
        .map(|(id, _)| *id);
    let ep = Episode {
        source: format!("ticket:AGN-{n}"),
        project: Some("p-eval".into()),
        project_name: Some("eval".into()),
        outcome: "Reopened by the review (reopen #1): see the review comments".into(),
        reopened: true,
        transcript: c.transcript.clone(),
        run: Some(run),
        used: used.clone(),
        shared_used: c
            .shared
            .iter()
            .map(|(id, t)| (*id, (*t).to_owned()))
            .collect(),
        signals: vec!["the review reopened the work (reopen #1)".into()],
    };
    let clean = |t: &str| Some(t.to_owned());
    let started = Instant::now();
    let r = learn(
        provider.as_ref(),
        &pipeline_model(&model),
        &store,
        e.as_ref(),
        &ep,
        &clean,
    )
    .await;
    let (got_own, got_shared, error) = match &r {
        Ok(rep) => (rep.misleading.clone(), rep.misleading_shared.clone(), None),
        Err(e) => (vec![], vec![], Some(e.to_string())),
    };
    let want_own: Vec<i64> = culprit_own.into_iter().collect();
    let want_shared: Vec<i64> = culprit_shared.into_iter().collect();
    // A shared culprit's blame on an own lesson, or an own culprit's on a shared one.
    let cross = (c.kind == Kind::SharedFollowed && !got_own.is_empty())
        || (c.kind == Kind::OwnFollowed && !got_shared.is_empty());
    Ok(json!({
        "case": n,
        "kind": format!("{:?}", c.kind),
        "quoted": c.quoted,
        "topic": c.topic,
        "own": used,
        "shared": c.shared,
        "want_own": want_own,
        "want_shared": want_shared,
        "got_own": got_own,
        "got_shared": got_shared,
        "correct": error.is_none() && got_own == want_own && got_shared == want_shared,
        "cross": cross,
        "false_blame": got_own.iter().any(|x| !want_own.contains(x)) || got_shared.iter().any(|x| !want_shared.contains(x)),
        "missed": want_own.iter().any(|x| !got_own.contains(x)) || want_shared.iter().any(|x| !got_shared.contains(x)),
        "error": error,
        "tokens": r.as_ref().map(|r| r.tokens_in + r.tokens_out).unwrap_or(0),
        "ms": started.elapsed().as_millis() as u64,
    }))
}

/// `attribution`: `cases` random cases from `seed`, four at a time.
pub async fn attribution(
    provider: Arc<dyn Provider>,
    model: String,
    cases: usize,
    seed: u64,
) -> anyhow::Result<TaskResult> {
    let start = Instant::now();
    let mut rng = Rng(seed);
    let mut set = tokio::task::JoinSet::new();
    let gate = Arc::new(tokio::sync::Semaphore::new(4));
    for n in 1..=cases {
        let c = case(&mut rng, n);
        let (p, m, g) = (provider.clone(), model.clone(), gate.clone());
        set.spawn(async move {
            let _permit = g.acquire_owned().await;
            attribute(p, m, c, n).await
        });
    }
    let mut out = Vec::new();
    while let Some(r) = set.join_next().await {
        out.push(r??);
    }
    out.sort_by_key(|v| v["case"].as_u64());
    let count = |f: &dyn Fn(&Value) -> bool| out.iter().filter(|v| f(v)).count();
    let correct = count(&|v| v["correct"] == true);
    let cross = count(&|v| v["cross"] == true);
    let errors = count(&|v| !v["error"].is_null());
    let mut by_kind = serde_json::Map::new();
    for k in KINDS {
        let name = format!("{k:?}");
        let of: Vec<&Value> = out.iter().filter(|v| v["kind"] == name.as_str()).collect();
        by_kind.insert(
            name,
            json!({ "cases": of.len(), "correct": of.iter().filter(|v| v["correct"] == true).count(),
                    "false_blame": of.iter().filter(|v| v["false_blame"] == true).count(),
                    "missed": of.iter().filter(|v| v["missed"] == true).count() }),
        );
    }
    // The id mapping is the code's part: no cross blame and no failed call. How often the model names the right
    // memory is its own judgment, recorded per kind; below 80 % the scenario fails too.
    let passed = cross == 0 && errors == 0 && correct * 5 >= cases * 4;
    Ok(TaskResult {
        id: format!("{ID}-attribution"),
        passed,
        closed: true,
        detail: format!("{correct}/{cases} attributed right, {cross} cross blame(s), {errors} failed call(s), seed {seed}"),
        runs: cases,
        steps: 0,
        tool_errors: errors as u32,
        error_codes: Default::default(),
        errors: out.iter().filter_map(|v| v["error"].as_str().map(str::to_owned)).collect(),
        tools: Default::default(),
        outcomes: vec![],
        reopened: 0,
        tokens_in: out.iter().map(|v| v["tokens"].as_u64().unwrap_or(0)).sum(),
        tokens_cached: 0,
        tokens_out: 0,
        cost_usd: 0.0,
        elapsed_ms: start.elapsed().as_millis() as u64,
        record: None,
        learning: Some(json!({ "seed": seed, "by_kind": by_kind, "cases": out })),
        discipline: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cases of a seed are fixed, cover every kind, and their answers are well formed: a culprit's text is shown
    /// exactly once, on its side; a shared id is unique within its case.
    #[test]
    fn random_cases_are_reproducible_and_well_formed() {
        let draw = |seed| {
            let mut r = Rng(seed);
            (1..=60).map(|n| case(&mut r, n)).collect::<Vec<_>>()
        };
        let (a, b) = (draw(7), draw(7));
        assert!(a
            .iter()
            .zip(&b)
            .all(|(x, y)| x.transcript == y.transcript && x.shared == y.shared));
        for k in KINDS {
            assert!(a.iter().any(|c| c.kind == k), "{k:?}");
        }
        for c in &a {
            let wrong = TOPICS[c.topic].wrong;
            let on_own = c.own.iter().filter(|(t, _)| *t == wrong).count();
            let on_shared = c.shared.iter().filter(|(_, t)| *t == wrong).count();
            match c.kind {
                Kind::OwnFollowed | Kind::OwnNotFollowed => assert_eq!((on_own, on_shared), (1, 0)),
                Kind::SharedFollowed | Kind::SharedNotFollowed => {
                    assert_eq!((on_own, on_shared), (0, 1))
                }
                Kind::Unrelated => assert_eq!((on_own, on_shared), (0, 0)),
            }
            let mut ids: Vec<i64> = c.shared.iter().map(|(i, _)| *i).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), c.shared.len());
            assert!(!c.own.is_empty() || !c.shared.is_empty());
        }
        // Pool ids collide with the agent's own (1, 2, 3) in a good share of the cases.
        assert!(
            a.iter()
                .filter(|c| c.shared.iter().any(|(i, _)| (*i as usize) <= c.own.len()))
                .count()
                >= 10
        );
    }
}
