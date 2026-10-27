# Reverse engineering by target type

The workflow is the same (triage → static → dynamic → Evidence→Finding→Path); the toolchain and the pitfalls differ
by target. Pick the section that matches the artifact. Authorized copies only, inside the sandbox.

## Native (ELF / PE / Mach-O)
- **Triage:** `file`, architecture (x86/x64/ARM/MIPS), `strings`, packing (UPX and friends), symbols, imports.
- **Static:** Ghidra / IDA / radare2; follow cross-references to the sinks (auth checks, crypto, parsers, update paths).
- **Dynamic:** gdb (+pwndbg/gef), or emulate with Qiling/Unicorn when you cannot run the target natively.
- **Pitfalls:** stripped binaries; anti-debug and anti-VM; packed code that only unpacks at runtime (dump then analyse).

## Managed (.NET / Java)
- **Triage:** confirm it is managed (CLR/JVM), note the obfuscator (ConfuserEx, SmartAssembly, ProGuard).
- **Static:** dnSpyEx / ILSpy for .NET, jadx / CFR for Java; de-obfuscate first (de4dot) so the decompilation is readable.
- **Dynamic:** set breakpoints in dnSpy/the debugger; dump decrypted strings and resources at runtime.
- **Pitfalls:** control-flow flattening and string encryption; reflection hiding the real call targets.

## Mobile (APK / IPA)
- **Triage:** unpack the package; read the manifest, permissions and the native libs it bundles.
- **Static:** jadx for the Dalvik code; analyse bundled `.so` as a native target (above).
- **Dynamic:** Frida/Objection to hook methods, bypass root/jailbreak and SSL-pinning checks, and read values at runtime.
- **Pitfalls:** logic split between Java and native; packers; integrity/anti-hook checks.

## Protocol / file format
- **Triage:** capture or collect samples; diff several to find fixed vs variable fields.
- **Static:** map the structure field by field (type, length, endianness, checksums, framing).
- **Dynamic:** replay a captured exchange and mutate one field at a time to confirm each field's meaning.
- **Pitfalls:** encryption or compression wrapping the real payload; a checksum that must be recomputed to replay.
