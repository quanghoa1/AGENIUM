#!/usr/bin/env python3
"""Lint a commit message. Usage: python check_commit.py <file>  (or pipe the message on stdin).
Checks: subject <= 72 chars, imperative-ish mood, blank line before body, body wrapped <= 72.
Advisory only: it prints warnings and always exits 0."""
import sys

def read():
    if len(sys.argv) > 1:
        with open(sys.argv[1], encoding="utf-8") as f:
            return f.read()
    return sys.stdin.read()

NON_IMPERATIVE = ("added", "fixed", "updated", "removed", "changed", "fixes",
                  "fixing", "adds", "adding", "refactored", "implemented")

def main():
    text = read().rstrip("\n")
    lines = text.split("\n")
    warn = []
    if not lines or not lines[0].strip():
        warn.append("empty subject line")
        print("\n".join("- " + w for w in warn) or "ok")
        return
    subject = lines[0].rstrip()
    if len(subject) > 72:
        warn.append(f"subject is {len(subject)} chars (keep <= 72)")
    first = subject.split(" ", 1)[0].lower().rstrip(":")
    if first in NON_IMPERATIVE:
        warn.append(f"subject starts with '{first}'; use the imperative ('Add', not 'Added')")
    if subject.endswith("."):
        warn.append("subject ends with a period")
    if len(lines) > 1 and lines[1].strip():
        warn.append("no blank line between subject and body")
    for i, ln in enumerate(lines[2:], start=3):
        if len(ln) > 72 and not ln.lstrip().startswith(("http", "|", "```")):
            warn.append(f"body line {i} is {len(ln)} chars (wrap at ~72)")
    print("\n".join("- " + w for w in warn) if warn else "ok: the message looks clean")

if __name__ == "__main__":
    main()
