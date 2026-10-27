---
name: firmware-analysis
description: Analyse a firmware image on an authorized copy: unpack it, map the filesystem and services, and find stored secrets, weak update/transport paths and exposed interfaces. Use for router/IoT/device firmware and embedded images. (Learned from open firmware RE skill sets; our own workflow.)
tags: pentest firmware iot
---

## Job
Unpack a firmware image and find where the device it runs is weak, working only on the copy the engagement provides.

## When to use
A ticket provides a firmware image or a device dump to assess. Authorized artifacts only; never flash or touch a device outside the agreed scope.

## Steps
1. **Identify and carve.** `binwalk` the image: note the bootloader, kernel and filesystem type and offsets. Extract the root filesystem (binwalk/`unsquashfs`/`jefferson`/`ubireader` as the type needs) into the worktree.
2. **Map it.** Walk the filesystem: init scripts and services started at boot, the web/admin interface, listening daemons, busybox applets, and the update mechanism.
3. **Find the weaknesses.** Look for stored secrets (keys, certs, hardcoded credentials, `/etc/shadow`), world-writable or setuid binaries, an unauthenticated or signature-less update path, cleartext transport, and debug/backdoor interfaces. For a binary that looks decisive, hand it to `reverse-engineering`.
4. **Conclude Evidence → Finding → Path.** Each weakness: the file and offset, what it is, and the path it opens (local priv-esc, auth bypass, firmware replacement). Hand off to the app/web tester whatever confirms end to end.

## Tools
`shell` for the extraction and analysis toolchain (install only with the user's permission, ADR-120); `read_file` for extracted files. Keep the image and everything carved from it inside the ticket's worktree and the sandbox.

## Proof
The image is unpacked and its services mapped; each weakness is Evidence → Finding → Path with the file and offset. State what unpacked cleanly and what resisted; never claim a secret or a path you did not locate in the image.
