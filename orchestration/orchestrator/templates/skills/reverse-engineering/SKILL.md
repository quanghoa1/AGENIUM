---
name: reverse-engineering
description: Reverse engineer a binary, managed assembly or protocol on an authorized artifact: triage, recover the structure statically then dynamically, and hand a tester the weakness with evidence. Use for native/.NET/mobile binaries, firmware blobs and unknown protocols. (Learned from open RE skill sets; our own workflow.)
tags: pentest reverse-engineering binary
---

## Job
Recover enough of an artifact's structure to show where it is weak, working only on the copy the engagement provides.

## When to use
A ticket needs a binary, managed assembly, mobile package or wire protocol understood — formats, keys, checks, or a path a tester can then confirm. Authorized artifacts only.

## Steps
1. **Triage.** Identify the file: `file`, `strings`, size, packing/obfuscation, architecture, symbols. For .NET note the obfuscator; for mobile unpack the package; for a protocol capture a sample exchange. Decide static-first or dynamic-first.
2. **Static.** Recover structure without running it: a disassembler/decompiler (r2/Ghidra/IDA, dnSpy for .NET, jadx for APK), cross-references, data flow to the interesting sinks (auth checks, crypto, parsers, update/transport). The toolchain and pitfalls differ by target (native / managed / mobile / protocol) — `references/targets.md` has each. Note each finding with the address or offset.
3. **Dynamic, only where the rules allow and in a sandbox.** Run under a debugger or hook (gdb/Frida/Qiling) to confirm a path, dump a decrypted buffer, or watch a check at runtime. Never run an untrusted sample outside isolation.
4. **Conclude Evidence → Finding → Path.** For each weakness: the evidence (offset, decompiled snippet, runtime trace), what it is (e.g. hardcoded key, missing signature check, parser overflow), and the path it opens for a tester. Hand it off; do not keep raw dumps in the report.

## Tools
`shell` for the toolchain the project has (install only with the user's permission, ADR-120); `read_file`/`write_file` within the ticket's worktree for notes and extracted structure. Keep the sample and your work inside the sandbox.

## Proof
Every artifact in scope is triaged and each weakness is stated as Evidence → Finding → Path with a reproducible pointer (offset/address/trace). Say what you recovered and what stayed opaque; never infer a weakness you did not evidence.
