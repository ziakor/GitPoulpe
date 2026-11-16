#!/usr/bin/env python3
"""Measure frame pacing in the real window on the reference repository.

Usage: tools/measure-reference.py [--repo PATH] [--runs N] [--seconds S] [--speed PX_PER_S]

Needs the release binary (cargo build --release -p gitpoulpe-desktop). Opens the window, scrolls it
at a constant speed for S seconds, records the interval between frames and writes a report. The
window must stay visible and focused: a hidden window is not asked for frames and the run is
reported as invalid.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def run_once(binary: str, repo: str, mode: str, seconds: int, speed: int) -> dict:
    with tempfile.TemporaryDirectory() as prefs, tempfile.TemporaryDirectory() as out:
        with open(os.path.join(prefs, "prefs.json"), "w") as handle:
            json.dump({"diff_mode": mode}, handle)
        report = os.path.join(out, "report.json")
        env = dict(os.environ, GITPOULPE_PERF_LOG=report, GITPOULPE_AUTOSCROLL=str(seconds),
                   GITPOULPE_SCROLL_SPEED=str(speed), RUNE_PREFS_DIR=prefs, RUNE_PERF_WARN="0")
        subprocess.run([binary, repo], env=env, timeout=seconds + 60, check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if not os.path.exists(report):
            return {"validity": "invalid: no report was written"}
        with open(report) as handle:
            return json.load(handle)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--repo", default=os.path.expanduser("~/bench/gitpoulpe-reference"))
    parser.add_argument("--binary", default=os.path.join(ROOT, "target/release/gitpoulpe"))
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--seconds", type=int, default=20)
    parser.add_argument("--speed", type=int, default=3000)
    args = parser.parse_args()
    if not os.path.exists(args.binary):
        sys.exit(f"{args.binary} is missing: cargo build --release -p gitpoulpe-desktop")
    invalid = 0
    for mode in ("unified", "split"):
        for run in range(1, args.runs + 1):
            report = run_once(args.binary, args.repo, mode, args.seconds, args.speed)
            validity = report.get("validity", "invalid")
            if not str(validity).startswith("valid"):
                invalid += 1
                print(f"{mode:8} run {run}: {validity}")
                continue
            interval = report["interval_ms"]
            print(f"{mode:8} run {run}: {report['frames']} frames, interval p50 {interval['p50']} p95 {interval['p95']} "
                  f"p99 {interval['p99']} max {interval['max']} ms, over 25 ms: {report['over_25ms']}, "
                  f"over 33 ms: {report['over_33ms']}, idle gaps: {report['idle_gaps']}, "
                  f"first document at {report['ready_at_s']} s, {report['rss_kib'] / 1024:.0f} MiB")
    if invalid:
        print(f"\n{invalid} run(s) invalid: keep the window visible and focused during the whole run.")
    return 1 if invalid else 0


if __name__ == "__main__":
    sys.exit(main())
