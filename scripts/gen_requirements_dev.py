#!/usr/bin/env python
"""Generate requirements/dev.txt with --require-hashes compatible pinning.

Target: python:3.12-slim-bookworm on x86_64 (Dockerfile.dev L20).

Strategy: pick the SINGLE best wheel per package for the target
(cp312 + manylinux x86_64, or py3-none-any). pip --require-hashes is
strictly platform-specific — multi-hash files only work if every resolved
artifact's hash matches one in the list, but pip's hash check actually
fails on cross-platform mismatches because it picks the first hash to
verify against. Single-target hashing is the safe path.

Regenerate when any package version is bumped:
    python scripts/gen_requirements_dev.py
"""
import json
import os
import sys
import urllib.request

PKGS = [
    ("pip",         "26.2.1"),
    ("pytest",      "9.1.1"),
    ("pytest-cov",  "7.1.0"),
    ("pytest-xdist","3.8.0"),
    ("ruff",        "0.16.5"),
    ("mypy",        "2.3.1"),
    ("pip-audit",   "2.10.1"),
    ("build",       "1.6.0"),
    ("pyyaml",      "6.0.3"),
    ("requests",    "2.34.2"),
]

# Target platform strings to match in wheel filenames. First match wins.
TARGET_PATTERNS = [
    "cp312-cp312-manylinux2014_x86_64",   # mypy / pyyaml cp312 x86_64 wheel
    "py3-none-manylinux_2_17_x86_64",     # ruff manylinux x86_64
    "py3-none-any",                        # pip / pytest / etc. pure-python
]


def best_wheel(name, ver):
    """Return (sha256, filename) of the best-matching wheel for our target."""
    url = f"https://pypi.org/pypi/{name}/{ver}/json"
    with urllib.request.urlopen(url, timeout=20) as r:
        d = json.loads(r.read())
    files = [u for u in d.get("urls", [])
             if not u.get("yanked") and u.get("packagetype") == "bdist_wheel"]
    for pat in TARGET_PATTERNS:
        for f in files:
            if pat in f["filename"]:
                return f["digests"]["sha256"], f["filename"]
    # Fallback: any non-yanked wheel
    if files:
        return files[0]["digests"]["sha256"], files[0]["filename"]
    return None, None


HEADER = [
    "# Generated 2026-08-27 by forgecode Scorecard remediation. Closes the",
    "# last 3 PinnedDependencies findings (pipCommand not pinned by hash).",
    "#",
    "# Target platform: python:3.12-slim-bookworm on x86_64 (Dockerfile.dev L20).",
    "# pip install --require-hashes will refuse install if the resolved",
    "# artifact's sha256 does not match the --hash listed below.",
    "#",
    "# Verify by:",
    "#   pip install --no-cache-dir --require-hashes --dry-run \\",
    "#       -r requirements/dev.txt",
    "# Regenerate when any package version is bumped:",
    "#   python scripts/gen_requirements_dev.py",
    "",
]


def main():
    out = list(HEADER)
    failed = []
    for name, ver in PKGS:
        try:
            sha, fn = best_wheel(name, ver)
            if sha is None:
                failed.append((name, ver, "no wheel"))
                continue
            out.append(f"{name}=={ver}    --hash=sha256:{sha}    # {fn}")
            print(f"OK   {name}=={ver}  sha256={sha[:16]}...  file={fn}")
        except Exception as e:
            failed.append((name, ver, str(e)))
            print(f"FAIL {name}=={ver}: {e}")
    out.append("")
    os.makedirs("requirements", exist_ok=True)
    path = os.path.join("requirements", "dev.txt")
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(out))
    print(f"\nWROTE {path}  size={os.path.getsize(path)}  ok={not failed}")
    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
