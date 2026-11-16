#!/usr/bin/env python3
"""Build the M1 reference repository: 100,000 tracked files, 3,200 changed entries, one 50,000-line file.

Usage: tools/make-reference-repo.py <empty-target-directory>

Deterministic: the same arguments always produce the same tree. Changed entries:
2,900 modified, 100 deleted, 100 staged modifications, 50 untracked, 49 staged renames and
1 modified 50,000-line file, 3,200 in total.
"""
import os
import subprocess
import sys

TRACKED = 100_000
DIRS = 1_000
HUGE_LINES = 50_000
MODIFIED, DELETED, STAGED, UNTRACKED, RENAMED = 2_900, 100, 100, 50, 49
HUGE_CHANGES = 400


def small_file(directory: int, number: int) -> str:
    lines = [f"// module {directory:04}/{number:03}", "use crate::prelude::*;", ""]
    for k in range(6):
        lines += [
            f"pub fn item_{number:03}_{k}(value: usize) -> usize {{",
            f"    let scaled = value * {directory % 7 + k + 1};",
            f"    scaled + {number % 11}",
            "}",
            "",
        ]
    lines += [f"// end of {directory:04}/{number:03}"]
    return "\n".join(lines) + "\n"


def huge_file(variant: bool) -> str:
    out = []
    for i in range(HUGE_LINES):
        line = f"row {i:05}: the quick brown fox jumps over the lazy dog {i % 97}"
        if variant and i % (HUGE_LINES // HUGE_CHANGES) == 7:
            line = f"row {i:05}: CHANGED the quick brown fox jumps over the lazy dog {i % 89}"
        out.append(line)
    return "\n".join(out) + "\n"


def run(root: str, *args: str) -> None:
    env = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null",
               GIT_AUTHOR_NAME="reference", GIT_AUTHOR_EMAIL="reference@example.com",
               GIT_COMMITTER_NAME="reference", GIT_COMMITTER_EMAIL="reference@example.com")
    subprocess.run(["git", *args], cwd=root, env=env, check=True, stdout=subprocess.DEVNULL)


def write(root: str, relative: str, text: str) -> None:
    path = os.path.join(root, relative)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as handle:
        handle.write(text)


def small_paths():
    per_dir = (TRACKED - 1) // DIRS + 1
    count = 0
    for directory in range(DIRS):
        for number in range(per_dir):
            if count == TRACKED - 1:
                return
            yield directory, number
            count += 1


def main(root: str) -> None:
    if os.path.exists(root) and os.listdir(root):
        sys.exit(f"{root} is not empty")
    os.makedirs(root, exist_ok=True)
    run(root, "init", "-q", "-b", "main")
    paths = list(small_paths())
    for directory, number in paths:
        write(root, f"src/d{directory:04}/f{number:03}.rs", small_file(directory, number))
    write(root, "data/huge.txt", huge_file(False))
    run(root, "add", "-A")
    run(root, "commit", "-q", "-m", "reference base")

    names = [f"src/d{d:04}/f{n:03}.rs" for d, n in paths]
    used: set[str] = set()

    def fresh(count: int) -> list[str]:
        picked = []
        step = 0
        while len(picked) < count:
            candidate = names[(len(used) * 37 + step * 101) % len(names)]
            step += 1
            if candidate not in used:
                used.add(candidate)
                picked.append(candidate)
        return picked

    for relative in fresh(MODIFIED):
        full = os.path.join(root, relative)
        text = open(full).read()
        text = text.replace("pub fn item_", "pub fn changed_item_", 1) + "// touched\n"
        open(full, "w").write(text)
    for relative in fresh(DELETED):
        os.remove(os.path.join(root, relative))
    for relative in fresh(STAGED):
        full = os.path.join(root, relative)
        open(full, "a").write("// staged change\n")
        run(root, "add", relative)
    for relative in fresh(RENAMED):
        directory, name = os.path.split(relative)
        run(root, "mv", relative, os.path.join(directory, "renamed_" + name))
    for number in range(UNTRACKED):
        write(root, f"untracked/new_{number:03}.rs", small_file(9999, number))
    write(root, "data/huge.txt", huge_file(True))
    print(f"reference repository ready in {root}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
