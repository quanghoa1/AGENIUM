//! Report fields read the way models write them (ADR-142): a model that guesses a field's shape instead of reading the
//! schema — artifacts as plain paths, confidence as a word — gets its report through instead of one error per field
//! (measured: Claude Haiku 5.5 sent `submit_report` six times, each refused for the next field). Names it guesses
//! (`results`, `criterion`, `ticket`) are renamed before, with the other tools' arguments (`agenium_tools::coerce`). The
//! schema the model is shown stays the exact one.

use serde::de::Error;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::forms::ArtifactRef;

/// Confidence 0..1, also from a number in a string, a percentage ("90%", 90) or a word ("high", "medium", "low").
pub(crate) fn confidence<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = Value::deserialize(d)?;
    let n = match &v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t = s.trim().to_lowercase();
            match t.as_str() {
                "very high" | "certain" => Some(0.95),
                "high" => Some(0.9),
                "medium" | "moderate" => Some(0.6),
                "low" => Some(0.3),
                _ => t.trim_end_matches('%').trim().parse::<f64>().ok().map(|x| {
                    if t.ends_with('%') {
                        x / 100.0
                    } else {
                        x
                    }
                }),
            }
        }
        _ => None,
    }
    .ok_or_else(|| D::Error::custom(format!("confidence must be a number from 0 to 1, not {v}")))?;
    // A whole number over 1 is a percentage (90); 1.5 stays as written, for the report's check to refuse.
    Ok(if n > 1.0 && n <= 100.0 && n.fract() == 0.0 {
        n / 100.0
    } else {
        n
    })
}

/// Artifacts as objects (`{"kind", "ref"}`, `path` taken for `ref`) or as plain paths (a file).
pub(crate) fn artifacts<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ArtifactRef>, D::Error> {
    let v = Value::deserialize(d)?;
    let items = match v {
        Value::Null => return Ok(Vec::new()),
        Value::Array(a) => a,
        one => vec![one],
    };
    items
        .into_iter()
        .map(|it| match it {
            Value::String(s) => Ok(ArtifactRef {
                kind: "file".into(),
                reference: s,
            }),
            Value::Object(mut m) => {
                let reference = ["ref", "path", "reference", "file"]
                    .iter()
                    .find_map(|k| m.remove(*k))
                    .and_then(|r| r.as_str().map(str::to_owned))
                    .ok_or_else(|| {
                        D::Error::custom("an artifact needs its ref (a path, commit or log)")
                    })?;
                let kind = m
                    .remove("kind")
                    .or_else(|| m.remove("type"))
                    .and_then(|k| k.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "file".into());
                if let Some(k) = m.keys().next() {
                    return Err(D::Error::custom(format!(
                        "unknown field `{k}` in an artifact, expected `kind` and `ref`"
                    )));
                }
                Ok(ArtifactRef { kind, reference })
            }
            other => Err(D::Error::custom(format!(
                "an artifact is a path or {{kind, ref}}, not {other}"
            ))),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::forms::{AcStatus, WorkReport};
    use serde_json::json;

    #[test]
    fn a_report_written_from_a_guess_of_its_shape_is_read() {
        // What Claude Haiku 5.5 sent, field after field (each refused before).
        let r: WorkReport = serde_json::from_value(json!({
            "ticket_id": "AGN-1",
            "summary": "Wrote the summary.",
            "acceptance": [{"ac_id": "AC-1", "status": "met", "evidence": "deliverables/summary.txt"}],
            "artifacts": ["deliverables/summary.txt", {"path": "notes.md"}],
            "confidence": "High"
        }))
        .unwrap();
        assert_eq!(r.acceptance[0].ac_id, "AC-1");
        assert_eq!(r.acceptance[0].status, AcStatus::Met);
        assert_eq!(
            (
                r.artifacts[0].kind.as_str(),
                r.artifacts[0].reference.as_str()
            ),
            ("file", "deliverables/summary.txt")
        );
        assert_eq!(r.artifacts[1].reference, "notes.md");
        assert_eq!(r.confidence, 0.9);
        let r: WorkReport = serde_json::from_value(json!({
            "ticket_id": "AGN-1", "summary": "s",
            "acceptance": [{"ac_id": "AC-1", "status": "NotMet", "evidence": "e"}],
            "confidence": "85%"
        }))
        .unwrap();
        assert_eq!(r.confidence, 0.85);
        // Still refused: a field that means nothing, a confidence that is no number.
        assert!(serde_json::from_value::<WorkReport>(json!({
            "ticket_id": "AGN-1", "summary": "s", "acceptance": [], "confidence": 1, "mood": "good"
        }))
        .is_err());
        assert!(serde_json::from_value::<WorkReport>(json!({
            "ticket_id": "AGN-1", "summary": "s", "acceptance": [], "confidence": "sure-ish"
        }))
        .is_err());
    }
}
