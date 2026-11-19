#!/usr/bin/env python3
"""Checks that every dependency licence is compatible with GPL-3.0-only and that bundled assets carry theirs.

Usage: scripts/check-licenses.py [--list] [--self-test]
Needs cargo and the locked dependency set (Cargo.lock).
"""
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

ALLOWED = {
    "MIT", "MIT-0", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause",
    "ISC", "Zlib", "0BSD", "Unlicense", "CC0-1.0", "BSL-1.0", "Unicode-3.0", "MPL-2.0",
    "CDLA-Permissive-2.0", "LGPL-2.1-or-later", "GPL-3.0-only", "GPL-3.0-or-later",
}

TOKEN = re.compile(r"\(|\)|[A-Za-z0-9.+-]+")


def parse(expression):
    tokens = TOKEN.findall(expression.replace("/", " OR "))
    position = 0

    def peek():
        return tokens[position] if position < len(tokens) else None

    def take():
        nonlocal position
        if position >= len(tokens):
            raise ValueError(f"unexpected end of {expression!r}")
        position += 1
        return tokens[position - 1]

    def atom():
        if peek() == "(":
            take()
            value = disjunction()
            if peek() != ")":
                raise ValueError(f"unbalanced parentheses in {expression!r}")
            take()
            return value
        name = take()
        if peek() == "WITH":
            take()
            name = f"{name} WITH {take()}"
        return name in ALLOWED

    def conjunction():
        value = atom()
        while peek() == "AND":
            take()
            value = atom() and value
        return value

    def disjunction():
        value = conjunction()
        while peek() == "OR":
            take()
            value = conjunction() or value
        return value

    result = disjunction()
    if position != len(tokens):
        raise ValueError(f"cannot parse {expression!r}")
    return result


def self_test():
    cases = {
        "MIT": True,
        "MIT OR Apache-2.0": True,
        "Apache-2.0/MIT": True,
        "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT": True,
        "(Apache-2.0 OR MIT) AND BSD-3-Clause": True,
        "MIT AND SSPL-1.0": False,
        "SSPL-1.0 OR MIT": True,
        "GPL-2.0-only": False,
        "Proprietary": False,
        "AGPL-3.0-only AND MIT": False,
    }
    for expression, expected in cases.items():
        assert parse(expression) is expected, expression
    for broken in ("(MIT", "MIT OR", "MIT )"):
        try:
            parse(broken)
        except ValueError:
            continue
        raise AssertionError(f"{broken!r} should not parse")
    print("self-test passed")


def main():
    if "--self-test" in sys.argv:
        self_test()
        return 0
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--locked"],
            cwd=ROOT, check=True, capture_output=True, text=True,
        ).stdout
    )
    workspace = set(metadata["workspace_members"])
    problems = []
    rows = []
    for package in metadata["packages"]:
        expression = package.get("license")
        label = f"{package['name']} {package['version']}"
        if package["id"] in workspace:
            if expression != "GPL-3.0-only":
                problems.append(f"{label}: workspace crate must be GPL-3.0-only, found {expression!r}")
            continue
        if not expression:
            problems.append(f"{label}: no SPDX licence expression (licence file: {package.get('license_file')})")
            continue
        rows.append((package["name"], package["version"], expression))
        try:
            if not parse(expression):
                problems.append(f"{label}: {expression} is not in the GPL-3.0-compatible allowlist")
        except ValueError as error:
            problems.append(f"{label}: {error}")

    licence = (ROOT / "LICENSE").read_text()
    if "GNU GENERAL PUBLIC LICENSE" not in licence or "Version 3" not in licence:
        problems.append("LICENSE is not the GPL version 3 text")
    fonts = ROOT / "crates/gitpoulpe-desktop/assets/fonts"
    for font in sorted(fonts.glob("*.ttf")):
        family = "Sans" if "Sans" in font.name else "Mono"
        notice = fonts / f"LICENSE-IBM-Plex-{family}.txt"
        if not notice.is_file() or "SIL Open Font License" not in notice.read_text():
            problems.append(f"{font.name}: missing or wrong licence notice {notice.name}")

    if "--list" in sys.argv:
        for name, version, expression in sorted(rows):
            print(f"{name} {version}\t{expression}")
    print(f"{len(rows)} third-party crates checked, {len(problems)} problem(s)")
    for problem in problems:
        print(f"PROBLEM: {problem}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
