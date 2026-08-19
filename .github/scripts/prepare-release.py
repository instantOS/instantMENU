#!/usr/bin/env python3
"""Prepare the primary metadata and changelog for an instantMENU release.

This script owns release policy: selecting a semantic-version bump from commit
messages and updating files whose version is maintained by hand. Generated
artifacts are deliberately outside its scope. The version-bump workflow runs
instantmenu-mangen for the man page and makepkg for each .SRCINFO afterward.
"""

import argparse
import datetime as dt
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
Commit = tuple[str, str, str]


def run(args: list[str]) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def read(path: str) -> str:
    return (ROOT / path).read_text()


def write(path: str, text: str) -> None:
    (ROOT / path).write_text(text)


def current_version() -> str:
    match = re.search(r'^version = "([^"]+)"', read("Cargo.toml"), re.MULTILINE)
    if not match:
        raise SystemExit("failed to find package version in Cargo.toml")
    return match.group(1)


def latest_version_tag() -> str | None:
    tags = run(["git", "tag", "--merged", "HEAD", "--sort=-v:refname", "--list", "v[0-9]*"])
    return tags.splitlines()[0] if tags else None


def commits_since(tag: str | None) -> list[Commit]:
    git_range = f"{tag}..HEAD" if tag else "HEAD"
    raw = run(["git", "log", "--format=%H%x00%s%x00%b%x1e", git_range])
    commits = []
    for entry in raw.strip("\x1e").split("\x1e"):
        if not entry.strip():
            continue
        sha, subject, body = entry.lstrip("\n").split("\x00", 2)
        if subject.startswith(("chore: release v", "chore(release):")):
            continue
        commits.append((sha, subject.strip(), body.strip()))
    return commits


def bump_level(commits: list[Commit], requested: str) -> str | None:
    """Apply the repository's Conventional Commit release policy.

    Breaking changes bump major, features bump minor, maintenance-only commits
    do not release, and all other work—including non-conventional subjects—
    bumps patch. An explicit workflow input overrides this policy.
    """
    if requested != "auto":
        return requested
    level = None
    for _, subject, body in commits:
        if "BREAKING CHANGE" in body or re.match(r"^[a-zA-Z]+(?:\([^)]+\))?!:", subject):
            return "major"
        commit_type = subject.split(":", 1)[0].split("(", 1)[0].rstrip("!")
        if commit_type == "feat":
            level = "minor"
        elif commit_type not in {"chore", "ci", "docs", "style", "test"} and level != "minor":
            level = "patch"
    return level


def bump_version(version: str, level: str) -> str:
    major, minor, patch = map(int, version.split("."))
    if level == "major":
        return f"{major + 1}.0.0"
    if level == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def replace_once(path: str, pattern: str, replacement: str) -> None:
    text, count = re.subn(pattern, replacement, read(path), count=1, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"failed to update {path}")
    write(path, text)


def update_primary_versions(version: str) -> None:
    """Update source metadata; generated artifacts are handled by the workflow."""
    replace_once("Cargo.toml", r'^(version = )"[^"]+"', rf'\1"{version}"')
    replace_once(
        "Cargo.lock",
        r'(\[\[package\]\]\nname = "instantmenu"\nversion = )"[^"]+"',
        rf'\1"{version}"',
    )
    for path in ("packaging/arch/PKGBUILD", "packaging/arch-bin/PKGBUILD"):
        replace_once(path, r"^pkgver=.*", f"pkgver={version}")
        replace_once(path, r"^pkgrel=.*", "pkgrel=1")


def clean_subject(subject: str) -> tuple[str, str]:
    match = re.match(r"^(?P<type>[a-zA-Z]+)(?:\([^)]+\))?!?:\s*(?P<text>.+)$", subject)
    if not match:
        return "Other", subject
    kind, text = match.group("type"), match.group("text")
    if kind == "feat":
        return "Added", text
    if kind == "fix":
        return "Fixed", text
    if kind in {"perf", "refactor"}:
        return "Changed", text
    return "Other", text


def update_changelog(version: str, previous_tag: str | None, commits: list[Commit]) -> None:
    """Insert chronologically ordered, de-duplicated notes after Unreleased."""
    text = read("CHANGELOG.md")
    today = dt.date.today().isoformat()
    base = previous_tag or "HEAD"
    sections = [f"## [{version}](https://github.com/instantOS/instantMENU/compare/{base}...v{version}) - {today}"]
    groups = {"Added": [], "Changed": [], "Fixed": [], "Other": []}
    seen = set()
    for _, subject, _ in reversed(commits):
        group, line = clean_subject(subject)
        if line not in seen:
            groups[group].append(f"- {line}")
            seen.add(line)
    for group, lines in groups.items():
        if lines:
            sections.append(f"### {group}\n\n" + "\n".join(lines))
    marker = "## [Unreleased]"
    if marker not in text:
        raise SystemExit("failed to find Unreleased section in CHANGELOG.md")
    write("CHANGELOG.md", text.replace(marker, marker + "\n\n" + "\n\n".join(sections), 1))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bump", choices=["auto", "patch", "minor", "major"], default="auto")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    old_version = current_version()
    previous_tag = latest_version_tag()
    commits = commits_since(previous_tag)
    level = bump_level(commits, args.bump)
    if not level:
        print("No release-worthy commits found; nothing to do.")
        return 0
    new_version = bump_version(old_version, level)
    print(f"Bumping {old_version} -> {new_version} ({level})")
    if not args.dry_run:
        update_primary_versions(new_version)
        update_changelog(new_version, previous_tag, commits)
    return 0


if __name__ == "__main__":
    sys.exit(main())
