#!/usr/bin/env python3
"""Measure frame pacing in the real window on the reference repository.

Usage: tools/measure-reference.py [--repo PATH] [--runs N] [--seconds S]

Needs the release binary (cargo build --release -p gitpoulpe-desktop). Opens the window, scrolls it
at a constant 3,000 px/s for S seconds, records the interval between frames and writes a report. The
window must stay visible and focused: a hidden window is not asked for frames and the run is
reported as invalid.
"""
import argparse
import datetime
import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def run_once(binary: str, repo: str, mode: str, seconds: int) -> dict:
    with tempfile.TemporaryDirectory() as prefs, tempfile.TemporaryDirectory() as out:
        with open(os.path.join(prefs, "prefs.json"), "w") as handle:
            json.dump({"diff_mode": mode}, handle)
        report = os.path.join(out, "report.json")
        env = dict(os.environ, GITPOULPE_PERF_LOG=report, GITPOULPE_AUTOSCROLL=str(seconds),
                   RUNE_PREFS_DIR=prefs, RUNE_PERF_WARN="0")
        try:
            subprocess.run([binary, repo], env=env, timeout=seconds + 60, check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except subprocess.TimeoutExpired:
            return {"validity": "invalid: the window was never asked to draw frames, so the scroll never finished "
                                "(hidden window, screen asleep or session locked); the process was stopped"}
        if not os.path.exists(report):
            return {"validity": "invalid: no report was written"}
        with open(report) as handle:
            data = json.load(handle)
        if not data.get("autoscroll", {}).get("finished", False):
            data["validity"] = "invalid: the window was closed or the run interrupted before it finished"
        return data


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--repo", default=os.path.expanduser("~/bench/gitpoulpe-reference"))
    parser.add_argument("--binary", default=os.path.join(ROOT, "target/release/gitpoulpe"))
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--seconds", type=int, default=20)
    parser.add_argument("--results", default=os.path.expanduser("~/bench/results"))
    args = parser.parse_args()
    if not os.path.exists(args.binary):
        sys.exit(f"{args.binary} is missing: cargo build --release -p gitpoulpe-desktop")
    os.makedirs(args.results, exist_ok=True)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    log_path = os.path.join(args.results, f"{stamp}.jsonl")
    print(f"each run is saved in {log_path}; keep the window visible and focused, do not touch the mouse\n", flush=True)
    invalid = 0
    try:
        for mode in ("unified", "split"):
            for run in range(1, args.runs + 1):
                report = run_once(args.binary, args.repo, mode, args.seconds)
                with open(log_path, "a") as log:
                    log.write(json.dumps({"mode": mode, "run": run, "report": report}) + "\n")
                validity = report.get("validity", "invalid")
                if not str(validity).startswith("valid"):
                    invalid += 1
                    print(f"{mode:8} run {run}: {validity}", flush=True)
                    continue
                interval = report["interval_ms"]
                print(f"{mode:8} run {run}: {report['frames']} frames, interval p50 {interval['p50']} p95 {interval['p95']} "
                      f"p99 {interval['p99']} max {interval['max']} ms, over 25 ms: {report['over_25ms']}, "
                      f"over 33 ms: {report['over_33ms']}, idle gaps: {report['idle_gaps']}, "
                      f"first document at {report['ready_at_s']} s, {report['rss_kib'] / 1024:.0f} MiB, "
                      f"window {report.get('window', {}).get('width', '?')} wide, viewport {report.get('window', {}).get('viewport_height', '?')} high", flush=True)
    except KeyboardInterrupt:
        print(f"\ninterrupted; what was measured so far is in {log_path}")
        return 130
    if invalid:
        print(f"\n{invalid} run(s) invalid: keep the window visible and focused during the whole run.")
    print(f"\nresults: {log_path}")
    return 1 if invalid else 0


if __name__ == "__main__":
    sys.exit(main())
