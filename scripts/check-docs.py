#!/usr/bin/env python3
"""Checks the Markdown documentation: relative links and anchors resolve, and the command list in
docs/USER_GUIDE.md matches the palette commands of the application exactly.

Usage: scripts/check-docs.py
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = [ROOT / "README.md", ROOT / "CONTRIBUTING.md", *sorted((ROOT / "docs").glob("*.md"))]
LINK = re.compile(r"(?<!\!)\[[^\]]*\]\(([^)\s]+)\)")


def slug(heading):
    text = re.sub(r"[`*_]", "", heading.strip().lower())
    text = re.sub(r"[^\w\- ]", "", text, flags=re.UNICODE)
    return text.replace(" ", "-")


def anchors(path):
    found = set()
    in_code = False
    for line in path.read_text().splitlines():
        if line.startswith("```"):
            in_code = not in_code
        elif not in_code and line.startswith("#"):
            found.add(slug(line.lstrip("#")))
    return found


def check_links(problems):
    for doc in DOCS:
        in_code = False
        for number, line in enumerate(doc.read_text().splitlines(), 1):
            if line.startswith("```"):
                in_code = not in_code
            if in_code:
                continue
            for target in LINK.findall(line):
                if re.match(r"[a-z]+:", target):
                    continue
                name, _, fragment = target.partition("#")
                path = (doc.parent / name).resolve() if name else doc
                where = f"{doc.relative_to(ROOT)}:{number}"
                if not path.exists():
                    problems.append(f"{where}: {target} does not exist")
                elif fragment and path.suffix == ".md" and fragment not in anchors(path):
                    problems.append(f"{where}: no heading for #{fragment} in {path.relative_to(ROOT)}")


def palette_labels():
    source = (ROOT / "crates/gitpoulpe-desktop/src/palette.rs").read_text()
    return re.findall(r'label:\s*"([^"]+)"', source)


def guide_commands():
    text = (ROOT / "docs/USER_GUIDE.md").read_text()
    section = text.split("## Commands in the palette", 1)[1]
    return [line[2:].strip() for line in section.splitlines() if line.startswith("- ")]


def check_commands(problems):
    code, guide = palette_labels(), guide_commands()
    for label in code:
        if label not in guide:
            problems.append(f"palette command not documented: {label}")
    for label in guide:
        if label not in code:
            problems.append(f"documented command does not exist: {label}")
    if len(set(guide)) != len(guide):
        problems.append("duplicate command in the guide")


def main():
    problems = []
    check_links(problems)
    check_commands(problems)
    print(f"{len(DOCS)} documents checked, {len(palette_labels())} palette commands, {len(problems)} problem(s)")
    for problem in problems:
        print(f"PROBLEM: {problem}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
