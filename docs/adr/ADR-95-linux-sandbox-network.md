# ADR-95: Sandboxed commands get the network only as a private, outbound-only network — or none, and are told why

Status: Accepted (2026-10-26; the owner chose "safe, with fallback" over sharing the host network). Found on the first
run on a real Ubuntu 25.10 machine (VMware VM, kernel 6.17, bubblewrap 0.11, AppArmor userns restriction on). Amends
ADR-84 (network posture) for Linux; ADR-54 (bubblewrap backend).

## Context

ADR-84 made `[sandbox] network = true` the default. On Linux the backend is bubblewrap through `mxc-sdk`, which gives a
networked sandbox its own network namespace reached through slirp4netns. On the first real Ubuntu run:

1. **Every command with the network on failed to start**: our policy asked for `ingress.default = allow` and
   `ingress.hostLoopback = allow` (the same value as egress), and the bubblewrap backend refuses an accepting ingress
   posture. 1 test of 295 failed (the solo agent's shell step); in real use, no agent command and no CLI agent ran.
2. A private network needs **slirp4netns** and **unprivileged user + network namespaces**. Ubuntu 24.04+ restricts
   those by default (AppArmor, `kernel.apparmor_restrict_unprivileged_userns=1`); bwrap has its own AppArmor profile,
   the slirp/namespace setup does not. A bare `unshare --user --net` passes; the uid map then fails.
3. Where the namespace is allowed, **names do not resolve**: `/etc/resolv.conf` points at systemd-resolved's stub
   `127.0.0.53`, which inside the private namespace is the sandbox's own loopback. TCP by address works, DNS does not.
4. The only mode that works everywhere, sharing the host's network namespace, lets the sandbox reach every service on
   the host's loopback (databases, Agenium's own ports) and abstract unix sockets (X11), a real way out.

## Decision

- **Outbound only.** `network = true` opens egress; ingress and the host loopback stay denied in every case (Linux and
  macOS). Nothing connects into a sandbox, and a sandbox cannot reach host-local services.
- **A private network or none.** Before a command runs with the network on, Agenium checks once per process what
  mxc-sdk needs (slirp4netns on PATH; `unshare --user --map-current-user --keep-caps --net` succeeds). If not, the
  command runs **without network** and its output begins with one line saying so and why; it never runs with more
  access than asked for, and never with the host's network.
- **A CLI agent is refused, not degraded**: it cannot work without network, so starting one in that state fails with
  the reason and the fix.
- **DNS caveat**: when the private network works but resolv.conf names only a loopback stub, commands are told that
  names will not resolve, with the fix (`sudo ln -sf /run/systemd/resolve/resolv.conf /etc/resolv.conf`).
- **Told everywhere**: `agenium doctor` prints a `sandbox network:` line (private network / private but no DNS /
  without network and why); the work order tells the agent "no network on this machine: <why>" instead of "network on".
- **To enable** on Ubuntu 24.04+: install slirp4netns, allow unprivileged user namespaces (sysctl
  `kernel.apparmor_restrict_unprivileged_userns=0`, or an AppArmor profile for Agenium — to ship with the Linux
  installer), and point resolv.conf at the real resolver list. Verified on the VM: DNS and TCP out work, host loopback
  and unix sockets stay blocked, 0 escapes.

## Containment, measured on the shipping backend

`security/sandbox/examples/containment.rs` (+ `containment_payload.py`, our own code after the S3 attack list) runs a
payload through `Executor::run` with the worktree policy and decides escapes **on the host** (a write can succeed into a
private tmpfs without escaping): writes outside (outside dir, home, `..`, /tmp, main `.git` hooks and config, symlink,
hardlink, rename, delete, truncate), chmod/utime outside, reads (a secret outside, the real home and `~/.ssh`,
/etc/shadow, the host process's environ), the environment canary, network (host TCP listener, internet, DNS, host unix
socket), a double-forked `setsid` daemon after the run and after a timeout, and 7 things that must keep working.
`--baseline` runs it unsandboxed to show the payload detects escapes (all attacks escaped there except /etc/shadow and
the canary, which `clean_env` removes even without a sandbox).

Ubuntu 25.10, bubblewrap: **0 escapes** with the network off, on (fallback) and on (private network enabled); all
must-work items pass. Raw: `docs/reference/bench-raw/ubuntu-vm-2026-10-26/`. macOS Seatbelt: to be run (kit).

## Consequences

- `security/sandbox/src/unix.rs`: ingress/host loopback always deny; `private_network`, `private_dns_problem`,
  `effective` (fallback + note); `service.rs` refuses a networked contained service without a private network;
  `lib.rs`: `network_available`, `network_report`, test `a_command_with_the_network_on_starts`.
- `apps/cli` doctor line; `orchestration/orchestrator/src/runtime/mod.rs` work-order network note.
- Full suite on the VM: 296 passed, 0 failed (2 local-model tests ignored).
- Not done: shipping an AppArmor profile with a Linux installer; a DNS fix inside the sandbox that needs no host change
  (mxc-sdk cannot remap `/etc/resolv.conf`; worth raising upstream).
