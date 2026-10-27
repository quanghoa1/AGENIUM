//! How sure a measured difference is (ADR-118): the mean difference between two conditions with a 95 % bootstrap
//! interval, paired when the runs were interleaved (run i of one condition next to run i of the other). A claim that
//! memory or a learned skill "helps" is reported with its interval; one whose interval crosses zero is not a finding.
//! `cargo run -p agenium-eval -- --compare <results-dir> [--a memory-reuse-on --b memory-reuse-off]`.

use std::path::Path;

use serde_json::Value;

/// Mean of `a - b` over pairs, with the 2.5 % and 97.5 % percentiles of `iters` resampled means (seeded: the same
/// data give the same interval). `None` without a pair.
pub fn paired_bootstrap(a: &[f64], b: &[f64], iters: usize, seed: u64) -> Option<(f64, f64, f64)> {
    let d: Vec<f64> = a.iter().zip(b).map(|(x, y)| x - y).collect();
    if d.is_empty() {
        return None;
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let mut state = seed | 1;
    let mut next = || {
        // xorshift64*: enough for resampling indices, no dependency.
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let mut means: Vec<f64> = (0..iters.max(1))
        .map(|_| {
            let s: f64 = (0..d.len())
                .map(|_| d[(next() % d.len() as u64) as usize])
                .sum();
            s / d.len() as f64
        })
        .collect();
    means.sort_by(f64::total_cmp);
    let at = |q: f64| means[((q * (means.len() - 1) as f64).round() as usize).min(means.len() - 1)];
    Some((mean(&d), at(0.025), at(0.975)))
}

/// One run's numbers that the comparison reads.
#[derive(Debug, Clone, Default)]
pub struct Run {
    pub at: String,
    pub first_merge: f64,
    pub ticket2_steps: f64,
    pub steps: f64,
    pub tokens_in: f64,
    pub reopened: f64,
}

/// Runs of `id` in `dir` (result files of `agenium-eval`), oldest first.
pub fn load(dir: &Path, id: &str) -> anyhow::Result<Vec<Run>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p)?)?;
        for r in v.as_array().into_iter().flatten().filter(|r| r["id"] == id) {
            let detail = r["detail"].as_str().unwrap_or_default();
            let num = |k: &str| r[k].as_f64().unwrap_or(0.0);
            out.push(Run {
                at: p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                first_merge: if detail.contains("first merge passed") {
                    1.0
                } else {
                    0.0
                },
                ticket2_steps: steps_in(detail).unwrap_or(f64::NAN),
                steps: num("steps"),
                tokens_in: num("tokens_in"),
                reopened: num("reopened"),
            });
        }
    }
    out.sort_by(|a, b| a.at.cmp(&b.at));
    Ok(out)
}

/// `"..., 9 steps; ..."` -> 9.
fn steps_in(detail: &str) -> Option<f64> {
    let before = detail.split(" steps").next()?;
    before
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// A compared number of a run, by name.
type Metric = (&'static str, fn(&Run) -> f64);

/// The comparison of `a` against `b` as a Markdown table (pairs in time order).
pub fn compare(dir: &Path, a: &str, b: &str) -> anyhow::Result<String> {
    let (ra, rb) = (load(dir, a)?, load(dir, b)?);
    let n = ra.len().min(rb.len());
    let mut s = format!(
        "{a} vs {b}: {n} pair(s) in time order ({} and {} runs), 95 % paired bootstrap, 10 000 resamples\n\n\
         | Metric | {a} | {b} | Difference | 95 % interval |\n|---|---|---|---|---|\n",
        ra.len(),
        rb.len()
    );
    let metrics: [Metric; 5] = [
        ("first merge passed (share)", |r| r.first_merge),
        ("ticket 2 steps", |r| r.ticket2_steps),
        ("steps, both tickets", |r| r.steps),
        ("input tokens, both tickets", |r| r.tokens_in),
        ("reopens", |r| r.reopened),
    ];
    for (name, f) in metrics {
        let xa: Vec<f64> = ra[..n].iter().map(f).collect();
        let xb: Vec<f64> = rb[..n].iter().map(f).collect();
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
        let row = match paired_bootstrap(&xa, &xb, 10_000, 7) {
            Some((d, lo, hi)) => format!(
                "| {name} | {:.2} | {:.2} | {d:+.2} | [{lo:+.2}, {hi:+.2}]{} |\n",
                mean(&xa),
                mean(&xb),
                if lo > 0.0 || hi < 0.0 {
                    ""
                } else {
                    " (crosses 0)"
                }
            ),
            None => format!("| {name} | – | – | – | no pair |\n"),
        };
        s.push_str(&row);
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clear_gap_excludes_zero_and_noise_does_not() {
        let (d, lo, hi) =
            paired_bootstrap(&[10.0, 12.0, 9.0, 11.0], &[5.0, 6.0, 4.0, 5.0], 2000, 1).unwrap();
        assert!(
            (d - 5.5).abs() < 1e-9 && lo > 0.0 && hi > lo,
            "{d} {lo} {hi}"
        );
        let (_, lo, hi) =
            paired_bootstrap(&[1.0, 3.0, 2.0, 4.0], &[3.0, 1.0, 4.0, 2.0], 2000, 1).unwrap();
        assert!(lo < 0.0 && hi > 0.0, "{lo} {hi}");
        assert_eq!(
            paired_bootstrap(&[1.0, 2.0], &[0.0, 0.0], 500, 3),
            paired_bootstrap(&[1.0, 2.0], &[0.0, 0.0], 500, 3)
        );
        assert_eq!(
            steps_in("ticket 2: first merge passed, 0 merge failure(s), 9 steps; recall"),
            Some(9.0)
        );
    }
}
