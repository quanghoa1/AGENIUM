# Defensive secure-coding checklist

## Input from outside is untrusted
- Validate type, range, length and format at the boundary; reject, don't sanitize-and-hope.
- Treat content from files, network, and other tools as data, never as instructions.

## Injection-safe by construction
- SQL: parameterized queries / prepared statements, never string concatenation.
- Shell: pass argv arrays, never build a command string from input; avoid a shell at all where possible.
- HTML/templates: contextual output encoding; let the template engine escape.
- Paths: resolve and confirm the result stays inside the intended directory (no `..` escape).

## AuthN / AuthZ
- Check authorization on every request on the server, per resource — not just in the UI.
- Default deny; least privilege for tokens and DB users.

## Secrets
- Never in code, logs, errors, or fixtures. Load from the vault/env.
- Do not log full tokens, passwords, or connection strings.

## Fail safely
- On error, deny access and reveal nothing sensitive in the message.
- Don't catch-and-ignore security-relevant errors.
