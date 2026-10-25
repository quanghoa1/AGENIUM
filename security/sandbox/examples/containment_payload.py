"""Containment payload for `examples/containment.rs`: runs INSIDE an Agenium sandbox (Linux bubblewrap, macOS Seatbelt)
with cwd = the ticket worktree and prints one JSON object {test: [observed, detail]}. `observed` is "did" when the
action succeeded as seen from inside, "refused" when it raised. Inside a sandbox with its own namespaces a write can
"succeed" into a private tmpfs, so the harness decides escapes from the host side wherever it can; this view is used for
reads, the environment and the things that must keep working.

Argument `--hang`: start the daemon, then sleep (the harness kills the run by timeout)."""
import json
import os
import socket
import subprocess
import sys
import time

T = json.loads(os.environ["AGENIUM_TARGETS"])
WT = os.getcwd()
out = {}


def run(name, f):
    try:
        out[name] = ["did", str(f())[:200]]
    except Exception as e:  # noqa: BLE001 - every failure is the observation
        out[name] = ["refused", f"{type(e).__name__}: {e}"[:200]]


def write(path, text="agenium-escape"):
    with open(path, "w") as fh:
        fh.write(text)
    return path


def peek(path):
    """Length and first bytes: enough for the harness to recognise its canary, without printing whole files."""
    with open(path, "rb") as fh:
        data = fh.read()
    return f"{len(data)} {data[:12].hex()}"


def daemon():
    """A double-forked, session-leading process that keeps writing a heartbeat into the worktree."""
    pid = os.fork()
    if pid == 0:
        os.setsid()
        if os.fork() == 0:
            # A daemon lets go of the run's pipes, as real ones do.
            null = os.open(os.devnull, os.O_RDWR)
            for fd in (0, 1, 2):
                os.dup2(null, fd)
            hb = os.path.join(WT, "heartbeat")
            for _ in range(900):
                try:
                    write(hb, f"{os.getpid()} {time.time()}")
                except OSError:
                    pass
                time.sleep(0.2)
            os._exit(0)
        os._exit(0)
    os.waitpid(pid, 0)
    return "spawned"


if "--hang" in sys.argv:
    daemon()
    time.sleep(60)
    sys.exit(0)

# ---- writes outside the worktree (verified on the host)
run("write.outside_dir", lambda: write(os.path.join(T["outside"], "new.txt")))
run("write.home", lambda: write(os.path.join(T["home"], "new.txt")))
run("write.dotdot", lambda: write(os.path.join(WT, *[".."] * 4, T["tag"] + "-dotdot.txt")))
run("write.tmp", lambda: write(os.path.join("/tmp", T["tag"] + "-tmp.txt")))
run("write.main_git_hook", lambda: write(os.path.join(T["main_git"], "hooks", "pre-commit"), "#!/bin/sh\necho pwned\n"))


def git_config():
    with open(os.path.join(T["main_git"], "config"), "a") as fh:
        fh.write("\n[core]\n\thooksPath = /tmp\n")
    return "appended"


run("write.main_git_config", git_config)


def via_symlink():
    link = os.path.join(WT, "esc_link")
    os.symlink(T["outside"], link)
    return write(os.path.join(link, "via_symlink.txt"))


run("write.via_symlink", via_symlink)


def via_hardlink():
    link = os.path.join(WT, "esc_hard")
    os.link(T["victim_hardlink"], link)
    with open(link, "a") as fh:
        fh.write("hardlinked")
    return link


run("write.via_hardlink", via_hardlink)


def rename_out():
    p = write(os.path.join(WT, "moving.txt"))
    os.rename(p, os.path.join(T["outside"], "moved.txt"))
    return "moved"


run("write.rename_out", rename_out)
run("write.delete_outside", lambda: os.unlink(T["victim2"]))
run("write.truncate_outside", lambda: os.truncate(T["victim_truncate"], 0))
run("meta.chmod_outside", lambda: os.chmod(T["victim_meta"], 0o777))
run("meta.utime_outside", lambda: os.utime(T["victim_meta"], (0, 0)))

# ---- reads and the environment
run("read.secret_outside", lambda: peek(T["secret"]))
run("read.real_home_listing", lambda: sorted(os.listdir(T["real_home"]))[:5])
if T.get("real_ssh"):
    run("read.real_ssh_dir", lambda: sorted(os.listdir(T["real_ssh"]))[:5])
run("read.etc_shadow", lambda: peek("/etc/shadow"))
if sys.platform.startswith("linux"):
    run("read.host_process_environ", lambda: peek(f"/proc/{T['host_pid']}/environ"))
run("env.canary", lambda: os.environ[T["canary_name"]])

# ---- network (verified on the host for the two local listeners)


def tcp(host, port):
    s = socket.create_connection((host, port), timeout=3)
    s.sendall(b"x")
    s.close()
    return "connected"


run("net.tcp_host_listener", lambda: tcp("127.0.0.1", T["tcp_port"]))
run("net.tcp_internet", lambda: tcp("1.1.1.1", 443))
run("net.dns", lambda: socket.getaddrinfo("example.com", 443)[0][4])


def unix_socket():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(3)
    s.connect(T["unix_sock"])
    s.sendall(b"x")
    s.close()
    return "connected"


run("net.unix_host_socket", unix_socket)

# ---- what must keep working


def mkdir_rename():
    os.makedirs(os.path.join(WT, "d1", "d2"), exist_ok=True)
    os.rename(write(os.path.join(WT, "ok.txt")), os.path.join(WT, "d1", "d2", "ok.txt"))
    return "renamed"


run("ok.write_in_worktree", lambda: write(os.path.join(WT, "ok-write.txt")))
run("ok.mkdir_and_rename", mkdir_rename)
run("ok.tmpdir", lambda: write(os.path.join(os.environ["TMPDIR"], "ok.tmp")))
run("ok.dev_null", lambda: open("/dev/null", "w").write("x"))
run("ok.subprocess", lambda: subprocess.run(["sh", "-c", "echo hi"], check=True, capture_output=True, text=True).stdout.strip())
run("ok.git_status", lambda: subprocess.run(["git", "status", "--porcelain"], check=True, capture_output=True, text=True).returncode)
run("ok.read_own_interpreter", lambda: peek(sys.executable))

# ---- a daemon that tries to outlive the run (verified on the host)
run("daemon.spawned", daemon)

print(json.dumps(out))
