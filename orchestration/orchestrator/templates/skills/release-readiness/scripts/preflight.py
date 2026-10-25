#!/usr/bin/env python3
"""Read-only release preflight: reports whether the git tree is clean and which branch you are on.
Usage: python preflight.py   (run inside the repo). Advisory; it changes nothing and exits 0."""
import subprocess

def git(*args):
    try:
        return subprocess.run(["git", *args], capture_output=True, text=True, timeout=30).stdout.strip()
    except Exception as e:
        return f"(git failed: {e})"

def main():
    branch = git("rev-parse", "--abbrev-ref", "HEAD")
    status = git("status", "--porcelain")
    head = git("rev-parse", "--short", "HEAD")
    print(f"branch: {branch}")
    print(f"HEAD:   {head}")
    if status:
        n = len(status.splitlines())
        print(f"NO-GO: working tree has {n} uncommitted change(s):")
        print(status)
    else:
        print("GO: working tree is clean.")

if __name__ == "__main__":
    main()
