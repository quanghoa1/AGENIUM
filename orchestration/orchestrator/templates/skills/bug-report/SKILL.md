---
name: bug-report
description: Write a triager-grade security finding and self-check it before it is submitted: a minimal reproducible PoC, stated impact, and the per-class proof a triager expects. Use when a tester writes up a confirmed finding. (Learned from YesWeHack's reporting guidance; our own wording.)
tags: pentest security reporting
---

## Job
Turn a confirmed finding into a report a triager can reproduce and accept, and catch what a triager would reject before it is sent.

## When to use
A pentest or bug-bounty tester has a confirmed finding to write up, or a draft to check. Not for unconfirmed hunches — reproduce it first.

## Steps
1. **Scope first.** State the asset and that it is in the authorized scope and rules of engagement. Never infer scope from a domain name; if the scope is unclear, ask.
2. **Write the body in order:** Description (1–3 factual sentences, the class and where), Discovery (what you tried, dead ends kept), PoC, Steps to reproduce (atomic, preconditions stated: auth? role? victim interaction?), Impact (what an attacker gains, tied to this asset), Remediation.
3. **Make the PoC minimal and replayable:** a raw HTTP request/response or exact commands, only the requests needed, session tokens and PII replaced with `[REDACTED]`. A script only when raw requests cannot express the bug (real chains, races, many iterations).
4. **Self-triage** against `references/gotchas.md` for the claimed class: does the PoC meet the minimum proof; is this a common false positive; is the impact overclaimed? Give a verdict (READY / NEEDS FIXES / DO NOT SUBMIT) with concrete fixes.

## Tools
`read_file` for the draft and any captured requests; `shell` only to re-run the PoC and attach real output. Nothing here acts on a target — testing already happened under the engagement's rules.

## Proof
The finished report has every required section, a PoC a reader with no prior context can replay, and an impact tied to the asset. State the self-triage verdict and every fix applied. Never fabricate a request or an impact; mark what you could not verify.
