"""S2 harness (Windows sandbox through the installed Codex CLI). Spike code: throwaway; the results are the evidence.
Usage: python harness.py <out.json> [modes...]   (modes: mxc unelevated)"""
import ctypes, json, os, shutil, socket, subprocess, sys, threading, time, uuid
from ctypes import wintypes

HERE = os.path.dirname(os.path.abspath(__file__))
BASE = os.path.join(HERE, "run")  # a fresh sub-folder per fixture set (see fixtures())
PY = sys.executable
PROFILE = os.environ["USERPROFILE"]
DOCS = os.path.join(PROFILE, "Documents")
DESKTOP = os.path.join(PROFILE, "Desktop")
TOKEN = "agenium-s2-" + uuid.uuid4().hex[:8]


CODEX_HOME = os.path.join(HERE, "codex-home")


def codex_args(mode, extra=()):
    if mode == "mxc-profile":
        # Agenium-owned CODEX_HOME with the `agenium` permission profile (worktree write, toolchains read).
        return ["codex", "sandbox", "-c", 'windows.sandbox="mxc"', "-P", "agenium", *extra, "--"]
    return [
        "codex", "sandbox",
        "-c", f'windows.sandbox="{mode}"',
        "-c", 'sandbox_mode="workspace-write"',
        "-c", "sandbox_workspace_write.network_access=false",
        "-c", "sandbox_workspace_write.exclude_tmpdir_env_var=true",
        *extra,
        "--",
    ]


def sh(args, **kw):
    return subprocess.run(args, capture_output=True, text=True, **kw)


def fixtures():
    global BASE
    BASE = os.path.join(HERE, "run", uuid.uuid4().hex[:8])
    root = os.path.join(BASE, "agenium-root")
    os.makedirs(os.path.join(root, "db"))
    open(os.path.join(root, "config.toml"), "w").write("schema_version = 1\n")
    open(os.path.join(root, "db", "core.db"), "wb").write(b"SQLite format 3\x00" + b"\x00" * 100)
    proj = os.path.join(BASE, "project")
    os.makedirs(proj)
    sh(["git", "init", "-q", "-b", "main", proj])
    open(os.path.join(proj, "README.md"), "w").write("demo\n")
    sh(["git", "-C", proj, "add", "."])
    sh(["git", "-C", proj, "-c", "user.name=s2", "-c", "user.email=s2@x", "commit", "-qm", "init"])
    wts = {}
    for k in ["AGN-1", "AGN-2"]:
        p = os.path.join(proj, ".agenium", "worktrees", k)
        sh(["git", "-C", proj, "worktree", "add", "-q", "-b", f"agenium/{k}", p])
        wts[k] = p
    open(os.path.join(wts["AGN-2"], "secret.txt"), "w").write("other ticket\n")
    wt = wts["AGN-1"]
    r = sh(["cmd", "/c", "mklink", "/J", os.path.join(wt, "escape_junction"), root])
    assert r.returncode == 0, r.stdout + r.stderr
    os.makedirs(os.path.join(wt, ".tmp"))
    canary = os.path.join(DOCS, f"{TOKEN}-canary.txt")
    open(canary, "w").write("user document\n")
    return root, proj, wts, canary


def listener():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    s.listen(8)
    port = s.getsockname()[1]
    hits = []

    def run():
        while True:
            try:
                c, _ = s.accept()
                hits.append(1)
                c.close()
            except OSError:
                return

    threading.Thread(target=run, daemon=True).start()
    return s, port, hits


def env_for(wt, extra=None):
    e = dict(os.environ)
    e.update({
        "AGENIUM_S2_CANARY": "leak-me-123",
        "AGENIUM_FAKE_TOKEN": "sk-fake-000",
        "TEMP": os.path.join(wt, ".tmp"),
        "TMP": os.path.join(wt, ".tmp"),
    })
    e["CODEX_HOME"] = CODEX_HOME
    e.update(extra or {})
    return e


def cleanup(root, proj):
    leftovers = []
    for p in [os.path.join(DOCS, "agenium-s2-write.txt"), os.path.join(DESKTOP, "agenium-s2-write.txt"),
              os.path.join(PROFILE, "agenium-s2-write.txt"), os.path.join(DOCS, "agenium-s2-unc.txt"),
              os.path.join(DOCS, "agenium-s2-symlink.txt"), os.path.join(os.environ.get("TEMP", ""), "agenium-s2-write.txt")]:
        if os.path.exists(p):
            leftovers.append(p)
            os.remove(p)
    longdir = os.path.join(DOCS, "d" * 120)
    if os.path.exists("\\\\?\\" + longdir):
        leftovers.append(longdir)
        shutil.rmtree("\\\\?\\" + longdir, ignore_errors=True)
    for f in os.listdir(DOCS):
        if f.startswith(TOKEN):
            os.remove(os.path.join(DOCS, f))
    if sh(["schtasks", "/query", "/tn", "AgeniumS2Probe"]).returncode == 0:
        leftovers.append("scheduled task AgeniumS2Probe")
        sh(["schtasks", "/delete", "/tn", "AgeniumS2Probe", "/f"])
    import winreg
    try:
        k = winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Run", 0, winreg.KEY_ALL_ACCESS)
        winreg.QueryValueEx(k, "AgeniumS2Probe")
        winreg.DeleteValue(k, "AgeniumS2Probe")
        leftovers.append("Run key AgeniumS2Probe")
    except OSError:
        pass
    return leftovers


def attack_matrix(mode, root, proj, wts, canary, port):
    wt = wts["AGN-1"]
    targets = {
        "documents": DOCS, "desktop": DESKTOP, "agenium_root": root, "other_worktree": wts["AGN-2"],
        "project_git_config": os.path.join(proj, ".git", "config"), "profile_root": PROFILE,
        "temp_dir": os.environ.get("TEMP", "C:\\Windows\\Temp"), "ssh_dir": os.path.join(PROFILE, ".ssh"),
        "documents_canary": canary, "local_port": port,
    }
    shutil.copy(os.path.join(HERE, "payload.py"), os.path.join(wt, "s2_payload.py"))
    open(os.path.join(wt, "targets.json"), "w").write(json.dumps(targets))
    t = time.time()
    r = subprocess.run(codex_args(mode) + [PY, "s2_payload.py"], cwd=wt, env=env_for(wt), capture_output=True, text=True, timeout=300)
    elapsed = time.time() - t
    line = next((l for l in r.stdout.splitlines() if l.startswith("S2RESULT ")), None)
    if line is None:
        return {"error": (r.stdout + r.stderr)[-2000:]}, elapsed
    res = json.loads(line[len("S2RESULT "):])
    time.sleep(16)  # the detached child sleeps 12 s then writes its marker
    marker = os.path.join(wt, "detached_marker.txt")
    res["persist.detached_survives_run"] = ["escaped", "marker written after the run ended"] if os.path.exists(marker) else ["blocked", "no marker after the run ended"]
    return res, elapsed


# ---- kill matrix -------------------------------------------------------------------------------
TREE = r'''
import os, subprocess, sys, time
tok = sys.argv[1]
depth = int(sys.argv[2])
if depth < 2:
    for _ in range(2):
        subprocess.Popen([sys.executable, __file__, tok, str(depth + 1)])
while True:
    open(os.path.join(os.getcwd(), f"hb-{os.getpid()}.txt"), "w").write(str(time.time()))
    time.sleep(0.3)
'''

k32 = ctypes.WinDLL("kernel32", use_last_error=True)


class JOBOBJECT_BASIC_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [("PerProcessUserTimeLimit", ctypes.c_int64), ("PerJobUserTimeLimit", ctypes.c_int64),
                ("LimitFlags", wintypes.DWORD), ("MinimumWorkingSetSize", ctypes.c_size_t),
                ("MaximumWorkingSetSize", ctypes.c_size_t), ("ActiveProcessLimit", wintypes.DWORD),
                ("Affinity", ctypes.c_size_t), ("PriorityClass", wintypes.DWORD), ("SchedulingClass", wintypes.DWORD)]


class IO_COUNTERS(ctypes.Structure):
    _fields_ = [(n, ctypes.c_uint64) for n in ["a", "b", "c", "d", "e", "f"]]


class JOBOBJECT_EXTENDED_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [("BasicLimitInformation", JOBOBJECT_BASIC_LIMIT_INFORMATION), ("IoInfo", IO_COUNTERS),
                ("ProcessMemoryLimit", ctypes.c_size_t), ("JobMemoryLimit", ctypes.c_size_t),
                ("PeakProcessMemoryUsed", ctypes.c_size_t), ("PeakJobMemoryUsed", ctypes.c_size_t)]


k32.CreateJobObjectW.restype = wintypes.HANDLE


def job_for(proc):
    h = k32.CreateJobObjectW(None, None)
    info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION()
    info.BasicLimitInformation.LimitFlags = 0x2000  # KILL_ON_JOB_CLOSE
    k32.SetInformationJobObject(wintypes.HANDLE(h), 9, ctypes.byref(info), ctypes.sizeof(info))
    ok = k32.AssignProcessToJobObject(wintypes.HANDLE(h), wintypes.HANDLE(int(proc._handle)))
    return h, bool(ok)


def alive_with(token):
    r = sh(["powershell", "-NoProfile", "-Command",
            f"(Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*{token}*' -and $_.Name -ne 'powershell.exe' }}).ProcessId"])
    return [int(x) for x in r.stdout.split() if x.strip().isdigit()]


def kill_matrix(mode, wt, rounds=20):
    open(os.path.join(wt, "tree.py"), "w").write(TREE)
    results = []
    for i in range(rounds):
        tok = f"{TOKEN}-k{i}"
        for f in os.listdir(wt):
            if f.startswith("hb-"):
                os.remove(os.path.join(wt, f))
        p = subprocess.Popen(codex_args(mode) + [PY, "tree.py", tok, "0"], cwd=wt, env=env_for(wt),
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=0x00000200)
        job, assigned = job_for(p)
        deadline = time.time() + 30
        while time.time() < deadline and len([f for f in os.listdir(wt) if f.startswith("hb-")]) < 7:
            time.sleep(0.2)
        started = len([f for f in os.listdir(wt) if f.startswith("hb-")])
        # Agenium's executor: terminate the job, taskkill the tree, kill the child.
        k32.TerminateJobObject(wintypes.HANDLE(job), 1)
        sh(["taskkill", "/PID", str(p.pid), "/T", "/F"])
        p.kill()
        p.wait()
        k32.CloseHandle(wintypes.HANDLE(job))
        time.sleep(2)
        left = alive_with(tok)
        hb = {f: os.path.getmtime(os.path.join(wt, f)) for f in os.listdir(wt) if f.startswith("hb-")}
        time.sleep(1.5)
        still = [f for f, m in hb.items() if os.path.exists(os.path.join(wt, f)) and os.path.getmtime(os.path.join(wt, f)) > m]
        for pid in left:
            sh(["taskkill", "/PID", str(pid), "/T", "/F"])
        results.append({"round": i, "job_assigned": assigned, "processes_started": started, "orphans": left, "heartbeats_after_kill": len(still)})
    return results


# ---- valid work + overhead ------------------------------------------------------------------------
def crate(dir_):
    os.makedirs(os.path.join(dir_, "src"), exist_ok=True)
    open(os.path.join(dir_, "Cargo.toml"), "w").write('[package]\nname = "s2demo"\nversion = "0.1.0"\nedition = "2021"\n')
    body = "\n".join(f"pub fn f{i}(x: u64) -> u64 {{ (0..x).map(|v| v * {i} % 7).sum() }}" for i in range(6000))
    open(os.path.join(dir_, "src", "lib.rs"), "w").write(body + "\n#[cfg(test)] mod t { #[test] fn ok() { assert_eq!(super::f1(3), 3); } }\n")


def valid_work(mode, wt):
    crate(os.path.join(wt, "demo"))
    open(os.path.join(wt, "test_demo.py"), "w").write("import unittest\nclass T(unittest.TestCase):\n    def test_ok(self):\n        self.assertEqual(1 + 1, 2)\n")
    open(os.path.join(wt, "demo.js"), "w").write("console.log(require('os').platform(), [1,2,3].map(x => x * 2).join(','))\n")
    cargo = shutil.which("cargo") or os.path.join(PROFILE, ".cargo", "bin", "cargo.exe")
    jobs = {
        "git_status": ["git", "status", "--porcelain"],
        "git_log": ["git", "log", "--oneline", "-1"],
        "python_unittest": [PY, "-m", "unittest", "-q", "test_demo"],
        "node_script": [shutil.which("node") or "node", "demo.js"],
        "cargo_test": [cargo, "test", "--offline", "-q", "--manifest-path", "demo/Cargo.toml"],
    }
    out = {}
    for name, cmd in jobs.items():
        t = time.time()
        r = subprocess.run(codex_args(mode) + cmd, cwd=wt, env=env_for(wt, {"CARGO_TARGET_DIR": os.path.join(wt, "demo", "target")}), capture_output=True, text=True, timeout=600)
        out[name] = {"exit": r.returncode, "seconds": round(time.time() - t, 2), "tail": (r.stdout + r.stderr)[-300:]}
    return out


def overhead(mode, wt, runs=3):
    cargo = shutil.which("cargo") or os.path.join(PROFILE, ".cargo", "bin", "cargo.exe")
    tdir = os.path.join(wt, "demo", "target-bench")
    cmd = [cargo, "build", "--offline", "-q", "--manifest-path", "demo/Cargo.toml"]
    def once(sandboxed):
        shutil.rmtree(tdir, ignore_errors=True)
        t = time.time()
        r = subprocess.run((codex_args(mode) if sandboxed else []) + cmd, cwd=wt, env=env_for(wt, {"CARGO_TARGET_DIR": tdir}), capture_output=True, text=True, timeout=600)
        return time.time() - t, r.returncode
    rows = {"outside": [], "inside": []}
    for _ in range(runs):
        rows["outside"].append(once(False))
        rows["inside"].append(once(True))
    med = lambda v: sorted(x for x, _ in v)[len(v) // 2]
    st, t0 = [], []
    for _ in range(5):
        t = time.time()
        subprocess.run(codex_args(mode) + [PY, "-c", "pass"], cwd=wt, env=env_for(wt), capture_output=True)
        st.append(time.time() - t)
        t = time.time()
        subprocess.run([PY, "-c", "pass"], cwd=wt, env=env_for(wt), capture_output=True)
        t0.append(time.time() - t)
    return {"startup_inside_s": round(sorted(st)[2], 3), "startup_outside_s": round(sorted(t0)[2], 3),
            "outside_s": [round(x, 2) for x, _ in rows["outside"]], "inside_s": [round(x, 2) for x, _ in rows["inside"]],
            "exit_codes": [c for _, c in rows["inside"]], "overhead_pct": round((med(rows["inside"]) / med(rows["outside"]) - 1) * 100, 1)}


def main():
    out_path = sys.argv[1]
    modes = sys.argv[2:] or ["mxc", "unelevated"]
    report = {"codex": sh(["codex", "--version"]).stdout.strip(), "os": sh(["cmd", "/c", "ver"]).stdout.strip(),
              "token": TOKEN, "modes": {}}
    for mode in modes:
        root, proj, wts, canary = fixtures()
        lsock, port, hits = listener()
        try:
            m, secs = attack_matrix(mode, root, proj, wts, canary, port)
            m["net.localhost_listener_hits"] = ["escaped" if hits else "blocked", f"{len(hits)} connections reached the host listener"]
            rep = {"attack": m, "attack_seconds": round(secs, 1)}
            rep["leftovers_cleaned"] = cleanup(root, proj)
            rep["valid_work"] = valid_work(mode, wts["AGN-1"])
            rep["overhead"] = overhead(mode, wts["AGN-1"])
            rep["kill"] = kill_matrix(mode, wts["AGN-1"])
        finally:
            lsock.close()
            rep_left = cleanup(root, proj)
        rep["leftovers_after"] = rep_left
        report["modes"][mode] = rep
        json.dump(report, open(out_path, "w"), indent=1)
    print(json.dumps(report, indent=1)[:200])


if __name__ == "__main__":
    main()
