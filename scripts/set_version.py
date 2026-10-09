#!/usr/bin/env python3
"""Copy the number in VERSION into tauri.conf.json, Cargo.toml, Cargo.lock and the Flatpak metainfo.

    python3 scripts/set_version.py           apply
    python3 scripts/set_version.py --check   exit 1 if any file is out of sync
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
version = (ROOT / "VERSION").read_text().strip()
if not re.fullmatch(r"\d+\.\d+\.\d+", version):
    sys.exit(f"VERSION must look like 1.2.3, got {version!r}")

# (file, regex capturing the text around the old version, required); only the first match is replaced
TARGETS = [
    ("src-tauri/tauri.conf.json", r'("version":\s*")[^"]*(")', True),
    ("src-tauri/Cargo.toml", r'(?m)(^version\s*=\s*")[^"]*(")', True),
    ("src-tauri/Cargo.lock", r'(\[\[package\]\]\r?\nname = "dayfile"\r?\nversion = ")[^"]*(")', False),
    ("dev.dayfile.app.metainfo.xml", r'(<release version=")[^"]*(")', False),
]

check = "--check" in sys.argv
stale = []
for rel, pattern, required in TARGETS:
    path = ROOT / rel
    if not path.exists():
        if required:
            sys.exit(f"{rel}: file not found")
        continue
    with open(path, newline="") as f:
        text = f.read()
    new, n = re.subn(pattern, lambda m: m.group(1) + version + m.group(2), text, count=1)
    if n == 0:
        if required:
            sys.exit(f"{rel}: could not find the version line")
        continue
    if new == text:
        continue
    stale.append(rel)
    if not check:
        with open(path, "w", newline="") as f:
            f.write(new)

if check and stale:
    sys.exit(f"Out of sync with VERSION ({version}): {', '.join(stale)}\nRun: python3 scripts/set_version.py")
print(("Updated " + ", ".join(stale)) if stale else f"Already at {version}")
