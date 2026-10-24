"""S2 attack payload: runs INSIDE the sandbox with cwd = the ticket worktree. Prints one JSON object.
Each test returns (result, detail) where result is "escaped" (the attack worked), "blocked", or "ok"/"fail"
for tests that are supposed to work. The harness passes targets through AGENIUM_S2_TARGETS (JSON)."""
import json, os, socket, subprocess, sys, time

T = json.loads(os.environ.get("AGENIUM_S2_TARGETS") or open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "targets.json")).read())
out = {}


def attack(name, f):
    try:
        r = f()
        out[name] = ["escaped", str(r)[:160]]
    except Exception as e:
        out[name] = ["blocked", f"{type(e).__name__}: {e}"[:200]]


def must_work(name, f):
    try:
        out[name] = ["ok", str(f())[:160]]
    except Exception as e:
        out[name] = ["fail", f"{type(e).__name__}: {e}"[:200]]


def write(p):
    with open(p, "w") as fh:
        fh.write("agenium-s2")
    return p


def read(p):
    with open(p, "rb") as fh:
        return f"read {len(fh.read())} bytes"


# ---- writes outside the worktree
for k in ["documents", "desktop", "agenium_root", "other_worktree", "project_git_config", "profile_root", "temp_dir"]:
    attack(f"write.{k}", lambda k=k: write(os.path.join(T[k], "agenium-s2-write.txt") if k != "project_git_config" else T[k] + ".s2"))
attack("write.dotdot", lambda: write(os.path.join(os.getcwd(), "..", "..", "..", "agenium-s2-dotdot.txt")))
attack("write.unc_localhost", lambda: write("\\\\localhost\\" + T["documents"].replace(":", "$", 1) + "\\agenium-s2-unc.txt"))
attack("write.long_outside", lambda: write("\\\\?\\" + os.path.join(T["documents"], "d" * 120, "e" * 120, "agenium-s2-long.txt")) if os.makedirs("\\\\?\\" + os.path.join(T["documents"], "d" * 120, "e" * 120), exist_ok=True) is None else None)
attack("write.via_junction", lambda: write(os.path.join(os.getcwd(), "escape_junction", "agenium-s2-junction.txt")))


def make_junction():
    target = T["agenium_root"]
    link = os.path.join(os.getcwd(), "new_junction")
    r = subprocess.run(["cmd", "/c", "mklink", "/J", link, target], capture_output=True, text=True)
    if r.returncode != 0:
        raise PermissionError(r.stdout + r.stderr)
    return write(os.path.join(link, "agenium-s2-newjunction.txt"))


attack("write.via_new_junction", make_junction)


def make_symlink():
    link = os.path.join(os.getcwd(), "new_symlink")
    os.symlink(T["documents"], link, target_is_directory=True)
    return write(os.path.join(link, "agenium-s2-symlink.txt"))


attack("write.via_new_symlink", make_symlink)

# ---- reads outside the worktree
attack("read.ssh_dir", lambda: os.listdir(T["ssh_dir"]))
attack("read.agenium_config", lambda: read(os.path.join(T["agenium_root"], "config.toml")))
attack("read.agenium_db", lambda: read(os.path.join(T["agenium_root"], "db", "core.db")))
attack("read.documents_canary", lambda: read(T["documents_canary"]))
attack("read.other_worktree", lambda: read(os.path.join(T["other_worktree"], "secret.txt")))

# ---- network
def tcp(host, port, fam=socket.AF_INET):
    s = socket.socket(fam, socket.SOCK_STREAM)
    s.settimeout(5)
    s.connect((host, port))
    s.close()
    return f"connected {host}:{port}"


attack("net.tcp_ip_1.1.1.1_443", lambda: tcp("1.1.1.1", 443))
attack("net.tcp_ip_8.8.8.8_53", lambda: tcp("8.8.8.8", 53))
attack("net.tcp_ipv6", lambda: tcp("2606:4700:4700::1111", 443, socket.AF_INET6))
attack("net.dns_resolver", lambda: socket.getaddrinfo("example.com", 443)[0][4])


def udp_dns():
    q = b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01"
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.settimeout(5)
    s.sendto(q, ("8.8.8.8", 53))
    data, _ = s.recvfrom(512)
    return f"udp dns answer {len(data)} bytes"


attack("net.udp_dns_8.8.8.8", udp_dns)
attack("net.localhost_service", lambda: tcp("127.0.0.1", int(T["local_port"])))


def listen():
    s = socket.socket()
    s.bind(("0.0.0.0", 0))
    s.listen(1)
    p = s.getsockname()[1]
    s.close()
    return f"listening on 0.0.0.0:{p}"


attack("net.listen_all_interfaces", listen)

# ---- persistence
def detached():
    marker = os.path.join(os.getcwd(), "detached_marker.txt")
    code = f"import time; time.sleep(12); open(r'{marker}','w').write('alive')"
    flags = 0x00000008 | 0x00000200 | 0x01000000  # DETACHED_PROCESS | NEW_PROCESS_GROUP | BREAKAWAY_FROM_JOB
    try:
        p = subprocess.Popen([sys.executable, "-c", code], creationflags=flags, close_fds=True)
        return f"spawned breakaway pid {p.pid}"
    except OSError as e:
        p = subprocess.Popen([sys.executable, "-c", code], creationflags=0x00000008 | 0x00000200, close_fds=True)
        return f"breakaway refused ({e.winerror}); spawned detached pid {p.pid}"


must_work("persist.detached_spawn_attempt", detached)


def schtask():
    r = subprocess.run(["schtasks", "/create", "/tn", "AgeniumS2Probe", "/tr", "cmd /c exit", "/sc", "once", "/st", "23:59", "/f"], capture_output=True, text=True)
    if r.returncode != 0:
        raise PermissionError((r.stdout + r.stderr).strip())
    return "scheduled task created"


attack("persist.scheduled_task", schtask)


def run_key():
    import winreg
    k = winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Run", 0, winreg.KEY_SET_VALUE)
    winreg.SetValueEx(k, "AgeniumS2Probe", 0, winreg.REG_SZ, "cmd /c exit")
    return "Run key written"


attack("persist.registry_run_key", run_key)

# ---- environment leak
leak = {k: v for k, v in os.environ.items() if "CANARY" in k or k in ("OPENAI_API_KEY", "ANTHROPIC_API_KEY", "AGENIUM_FAKE_TOKEN")}
out["env.secrets_visible"] = ["escaped" if leak else "blocked", json.dumps(sorted(leak))]

# ---- valid work inside the worktree
must_work("work.write_inside", lambda: write(os.path.join(os.getcwd(), "inside.txt")))
must_work("work.long_path_inside", lambda: write("\\\\?\\" + os.path.join(os.getcwd(), "x" * 100, "y" * 100, "z" * 80 + ".txt")) if os.makedirs("\\\\?\\" + os.path.join(os.getcwd(), "x" * 100, "y" * 100), exist_ok=True) is None else None)
print("S2RESULT " + json.dumps(out))
