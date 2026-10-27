# Per-class proof and false-positive traps

For the claimed class: the **minimum proof** a PoC must show, the **common N/A** that triagers auto-close, and the
**overclaim traps** that need extra evidence. Apply only the section matching the finding's class. Authorized targets
only; everything here is for writing up a finding, not for widening an engagement.

## XSS
- **Minimum proof:** JS execution on the target origin (`alert(document.domain)`), and the injection context stated
  (HTML body / attribute / JS / URL / JSON).
- **Common N/A:** self-XSS with no delivery path; reflection without execution; a payload that only runs on a sandbox
  or `null` origin.
- **Overclaim traps:** "account takeover" needs the token-theft or action chain shown, not asserted.

## SQLi
- **Minimum proof:** a differential or error that proves query control (boolean/time-based diff, or an injected value
  changing the result), the parameter and the DBMS named.
- **Common N/A:** a WAF-blocked payload; an error message with no proven query control.
- **Overclaim traps:** "full DB dump" needs a column/table actually read, within scope and with no real PII exfiltrated.

## SSRF
- **Minimum proof:** an outbound request to a server you control from the target's infrastructure, with the collaborator
  hit shown.
- **Common N/A:** a redirect that never leaves the client; a request to a public host with no internal reach.
- **Overclaim traps:** "cloud metadata / internal access" needs the internal response shown, not just the outbound hit.

## IDOR / BOLA
- **Minimum proof:** two accounts — access another object with account A's session, show A is not authorized to it.
- **Common N/A:** same-account access; an unguessable id with no enumeration; data the user is meant to see.
- **Overclaim traps:** "mass exfiltration" needs the enumeration shown at small scale, with no real PII taken.

## CSRF
- **Minimum proof:** a cross-origin request that performs a state change with the victim's session and no secret the
  attacker lacks; the missing/forgeable token shown.
- **Common N/A:** a GET with no state change; an action behind a token the attacker cannot get; a logout CSRF of no value.

## RCE
- **Minimum proof:** command execution shown by a controlled, harmless marker (`id`, a unique DNS lookup); never a
  destructive command.
- **Common N/A:** a crash with no code execution; execution only in your own environment.
- **Overclaim traps:** stop at proof of execution; do not pivot or persist.

## SSTI / XXE / Open Redirect / Path Traversal
- **Minimum proof:** template evaluation of a marker (SSTI); an external entity resolving to a file or collaborator
  (XXE); a redirect to an attacker origin from a trusted one (Open Redirect); a file outside the web root read, with a
  harmless target (Traversal/LFI).
- **Common N/A:** reflection with no evaluation; an entity that never resolves; a redirect inside the same site; a path
  that stays in the web root.

## Auth bypass / Info disclosure / Race / CORS
- **Auth bypass:** reach a protected function without the credential it requires; show the control that failed.
- **Info disclosure:** a real secret (live key, another user's data) — a test/expired value is N/A; confirm it is live
  without abusing it.
- **Race:** the window exploited (e.g. double-spend, limit bypass) with the concurrent requests and the diverging
  result shown.
- **CORS:** `Access-Control-Allow-Origin` reflecting an attacker origin with credentials, and a read of a protected
  response shown — not just the header.

## Class not listed
Give the minimum proof yourself: the exact precondition, the action, and expected vs actual at the step where the bug
manifests. If it cannot be reproduced from scratch by someone with no context, it is not ready.
