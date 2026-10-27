---
name: recon-enumeration
description: Map the attack surface of authorized targets before testing: subdomains, live hosts, technologies, content and API endpoints, passive sources first. Use as the first step of a web pentest or bug-bounty engagement. (Learned from open offensive-security skill sets; our own workflow.)
tags: pentest recon osint
---

## Job
Build a complete, scoped map of what a target exposes, so testers know where to look before touching anything.

## When to use
The first step of a pentest or bug-bounty engagement, before vulnerability testing. Authorized targets only; report nothing outside the scope.

## Steps
1. **Confirm the scope** (in-scope domains, IPs, apps) and the rules; stay inside them. Prefer passive sources first.
2. **Enumerate hosts:** subdomains from passive sources, then resolve and probe for live HTTP services; note which respond and how.
3. **Fingerprint:** server, framework, versions, WAF/CDN, auth model, and anything that narrows the test (known CVEs, admin panels, dev/staging hosts).
4. **Discover content and endpoints:** directories and files the app does not link, parameters, and API routes — including endpoints and secrets referenced in the app's own JavaScript. Keep the active checks within what the rules allow.
5. **Deduplicate and rate** each asset with its source and confidence, and hand the map to the web/app testers (and tell the Lead), so testing starts from a known surface.

## Tools
`shell` for the recon toolchain (install only with the user's permission, ADR-120); `read_file`/`write_file` within the worktree for the asset list. Passive first; request permission before any active scan, and never probe a host outside the scope.

## Proof
Every in-scope target is mapped with its live hosts, technologies and discovered endpoints, each asset carrying its source and confidence. Say what was passive vs active. Never list or probe an asset outside the authorized scope.
