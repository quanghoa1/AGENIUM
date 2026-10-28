"""Generates the two labelled memory datasets (ADR-51 §13 items 3-4), deterministically:

  graph_qa.json      a synthetic operations world (services, components with known issues, teams, network vendors and
                     products) as ideal extractions, plus >= 100 questions labelled with the fact that answers each
                     (single-hop, two-hop through a service, two-hop through a team, scope traps with the facts that
                     must NOT be shown)
  entity_pairs.json  a catalogue of entities as stored in memory, plus ~2 000 mentions labelled with the entity each one
                     names (or none): surface variants, aliases, sibling products, other versions, conflicting vendor
                     scopes, other types and outsiders

Labels come from construction, not from a model. Run from anywhere: python -X utf8 -I make_datasets.py
The evaluators are ../graph_qa.rs and ../entity_resolution.rs.
"""

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
# -I leaves the script's folder off the path; the two generators live next to it
sys.path.insert(0, str(HERE))

from entity_catalogue import entity_pairs  # noqa: E402
from graph_world import graph_qa  # noqa: E402

def main():
    qa = graph_qa()
    (HERE / "graph_qa.json").write_text(json.dumps(qa, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    er = entity_pairs()
    (HERE / "entity_pairs.json").write_text(json.dumps(er, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    by = {}
    for q in qa["questions"]:
        by[q["type"]] = by.get(q["type"], 0) + 1
    pos = sum(1 for p in er["pairs"] if p["gold"])
    print(f"graph_qa.json: {qa['facts']} facts in {len(qa['episodes'])} episodes, {len(qa['questions'])} questions {by}")
    print(f"entity_pairs.json: {len(er['entities'])} entities, {len(er['pairs'])} pairs ({pos} same, "
          f"{len(er['pairs']) - pos} different)")


if __name__ == "__main__":
    main()
