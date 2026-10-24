"""Valid-work fixes under the Agenium profile: the repo's .git readable (worktree git dir), a sandbox-owned global
gitconfig, node with --preserve-symlinks-main. Prints one line per check."""
import os, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
runs = [os.path.join(HERE, "run", d) for d in os.listdir(os.path.join(HERE, "run")) if len(d) == 8]
R = max(runs, key=os.path.getmtime)
wt = os.path.join(R, "project", ".agenium", "worktrees", "AGN-1")
gitdir = os.path.join(R, "project", ".git")
home = os.path.join(HERE, "codex-home-fix")
os.makedirs(home, exist_ok=True)
base = open(os.path.join(HERE, "codex-home", "config.toml"), encoding="utf-8").read()
gc = os.path.join(HERE, "agent.gitconfig")
base = base.replace("[permissions.agenium.network]", f"'{gitdir}' = \"read\"\n'{gc}' = \"read\"\n\n[permissions.agenium.network]")
open(os.path.join(home, "config.toml"), "w", encoding="utf-8").write(base)
open(gc, "w").write("[user]\n\tname = agent\n\temail = agent@agenium.local\n")
env = dict(os.environ, CODEX_HOME=home, GIT_CONFIG_GLOBAL=gc, GIT_CONFIG_NOSYSTEM="1")
pre = ["codex", "sandbox", "-c", 'windows.sandbox="mxc"', "-P", "agenium", "--"]
checks = {
    "git status": (["git", "status", "--short"], {}),
    "git diff": (["git", "diff", "--stat"], {}),
    "git commit (agent should not commit)": (["git", "commit", "--allow-empty", "-qm", "x"], {}),
    "node plain": (["node", "demo.js"], {}),
    "node --preserve-symlinks-main": (["node", "demo.js"], {"NODE_OPTIONS": "--preserve-symlinks-main --preserve-symlinks"}),
}
for name, (cmd, extra) in checks.items():
    r = subprocess.run(pre + cmd, cwd=wt, env=dict(env, **extra), capture_output=True, text=True)
    tail = (r.stdout + r.stderr).strip().replace("\n", " | ")[-160:]
    print(f"{name:40s} exit={r.returncode} {tail}")
