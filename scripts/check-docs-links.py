#!/usr/bin/env python3
"""Check that relative links in the project docs point at real files.

Run from anywhere:

    python3 scripts/check-docs-links.py

Only relative links are checked; http(s), mailto, tel, and in-page anchors are
skipped. Links inside fenced code blocks are ignored. The script exits non-zero
and prints every broken link if any are found.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parent.parent
FILES = [
    ROOT / "README.md",
    ROOT / "SECURITY.md",
    ROOT / "CONTRIBUTING.md",
    ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md",
    *sorted((ROOT / "docs").glob("*.md")),
]

# [text](target) and ![alt](target); a title after whitespace is dropped below.
LINK = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
EXTERNAL = ("http://", "https://", "mailto:", "tel:", "ftp://")
FENCE = re.compile(r"^\s*(```|~~~)")


def links_in(path: Path) -> list[str]:
    """Return raw link targets outside fenced code blocks."""
    found: list[str] = []
    in_fence = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        for raw in LINK.findall(line):
            target = raw.strip()
            if target.startswith("<") and ">" in target:
                target = target[1 : target.index(">")]
            else:
                # Drop an optional Markdown title: [x](path "title")
                target = target.split(maxsplit=1)[0]
            if target:
                found.append(target)
    return found


def resolve(source: Path, target: str) -> Path | None:
    """Resolve a relative Markdown link; None means 'not our problem'."""
    if not target or target.startswith(EXTERNAL) or target.startswith("#"):
        return None
    path_part = unquote(target.split("#", 1)[0].split("?", 1)[0])
    if not path_part:
        return None
    if path_part.startswith("/"):
        # GitHub resolves a leading slash against the repository root.
        return ROOT / path_part.lstrip("/")
    return source.parent / path_part


def main() -> int:
    missing_files = [str(p.relative_to(ROOT)) for p in FILES if not p.is_file()]
    if missing_files:
        print("missing expected file(s): " + ", ".join(missing_files), file=sys.stderr)
        return 1

    broken: list[tuple[str, str, Path]] = []
    checked = 0
    for source in FILES:
        for target in links_in(source):
            resolved = resolve(source, target)
            if resolved is None:
                continue
            checked += 1
            if not resolved.exists():
                broken.append((str(source.relative_to(ROOT)), target, resolved))

    for source_name, target, resolved in broken:
        try:
            shown = resolved.relative_to(ROOT)
        except ValueError:
            shown = resolved
        print(f"broken link: {source_name} -> {target} (resolved to {shown})")

    if broken:
        print(f"\n{len(broken)} broken link(s); {checked} relative link(s) checked.")
        return 1

    print(f"ok: {checked} relative link(s) in {len(FILES)} file(s) resolve.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
