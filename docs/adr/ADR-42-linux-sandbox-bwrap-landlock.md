# ADR-42: Linux sandbox: bubblewrap preferred, Landlock ABI >= 3 required, fail-closed

- **Status:** Accepted (spike S3 decision, owner steering 10:40)
- **Date:** 2026-10-23 (Asia/Saigon)
- **Context / decision / consequences:**

- `Preference::Auto` picks bwrap when it really works (probed by running it), else the Landlock+seccomp helper when the kernel reports ABI >= 3 (ABI < 3 cannot stop `truncate`), else **refuses to run** (`SandboxError::Unavailable`).
- Executables are resolved to their real path before exec; the child gets a cleared env plus an allowlist; `AGENIUM_RUN_ID` marks processes for orphan cleanup.
- Known gap: Landlock backend allows chmod/utime outside writable roots; seccomp table is x86_64 only (aarch64 Linux is unsupported until a table is added — **open item**).
- Only simulated: ABI < 3 behaviour (no old kernel was available).
