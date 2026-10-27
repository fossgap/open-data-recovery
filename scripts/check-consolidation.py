#!/usr/bin/env python3
"""Validate and time the real optional consolidation feature on disposable images.

Uses the deterministic fixtures in consolidation_fixtures.py.
Needs separate /tmp and /dev/shm filesystems and the installed recovery engines.
"""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import tempfile
import time

import consolidation_fixtures as fixture_tools

ROOT = Path(__file__).resolve().parents[1]


def scan(binary, fixture, mode, workspace, consolidate, env, log):
    argv = [str(binary), "--scan", fixture["image"], "--workspace", str(workspace),
            "--mode", mode, "--extensions", fixture["extension"]]
    if consolidate:
        argv.append("--consolidate")
    started = time.perf_counter()
    result = subprocess.run(argv, capture_output=True, text=True, env=env, timeout=600)
    seconds = time.perf_counter() - started
    log.with_suffix(".stdout").write_text(result.stdout)
    log.with_suffix(".stderr").write_text(result.stderr)
    assert result.returncode == 0, result.stderr
    directory = Path(next(line[6:] for line in result.stdout.splitlines() if line.startswith("SAVED=")))
    manifest = json.loads((directory / "manifest.json").read_text())
    assert manifest["complete"] and not manifest["errors"]
    assert manifest["options"]["consolidate"] == consolidate
    assert bool(manifest.get("consolidation")) == consolidate
    return directory, manifest, seconds


def raw_rows(session):
    with fixture_tools.readonly_db(session / "tree.sqlite") as db:
        return [dict(row) for row in db.execute("SELECT * FROM candidates ORDER BY id")]


def validate(raw, grouped, fixture, binary, env):
    before, after = raw_rows(raw), raw_rows(grouped)
    # Eager extraction timestamps differ between runs; all discovery data agrees.
    fields = ("path", "bytes", "backend", "locator", "offset_bytes", "length_bytes", "artifact", "carved", "deleted")
    assert collections.Counter(tuple(c[f] for f in fields) for c in before) == \
        collections.Counter(tuple(c[f] for f in fields) for c in after)
    with fixture_tools.readonly_db(grouped / "tree.sqlite") as db:
        assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        groups = {r["id"]: dict(r) for r in db.execute("SELECT * FROM consolidated_candidates")}
        membership = {r["candidate_id"]: r["group_id"] for r in db.execute("SELECT * FROM consolidated_members")}
        saved_paths = collections.defaultdict(set)
        for r in db.execute("SELECT group_id,path FROM consolidated_paths"):
            saved_paths[r["group_id"]].add(r["path"])
    assert set(membership) == {c["id"] for c in after}
    manifest = json.loads((grouped / "manifest.json").read_text())
    by_digest, by_group = {}, {}
    expected_paths = collections.defaultdict(set)
    members = collections.Counter()
    for c in after:
        if c["artifact"]:
            digest = fixture_tools.sha(grouped / c["artifact"])
        else:
            # Independent extraction is outside the timed scan.
            result = subprocess.run(["icat", "-r", "-b", str(manifest["source"]["sector_size"]),
                                     "-o", str(c["offset_bytes"] // manifest["source"]["sector_size"]),
                                     fixture["image"], c["locator"]], check=True, capture_output=True)
            assert len(result.stdout) == c["bytes"]
            digest = hashlib.sha256(result.stdout).hexdigest()
        group = membership[c["id"]]
        assert by_digest.setdefault(digest, group) == group, "Missed exact duplicate"
        assert by_group.setdefault(group, digest) == digest, "Incorrect merge"
        expected_paths[group].add(c["path"])
        members[group] += 1
    assert expected_paths == saved_paths
    for id, g in groups.items():
        assert g["findings"] == members[id] and g["paths"] == len(saved_paths[id])
        if g["sha256"]:
            assert g["sha256"] == by_group[id]
            assert fixture_tools.sha(grouped / g["artifact"]) == g["sha256"]
    listed = fixture_tools.command([binary, "--list", grouped], env=env).stdout.splitlines()
    assert len(listed) == len(groups)
    if groups:
        group = max(groups.values(), key=lambda g: g["findings"])
        findings = fixture_tools.command([binary, "--findings", grouped, "--id", group["id"]], env=env).stdout.splitlines()
        assert len(findings) == group["findings"]
    assert len(by_digest) == len(groups)
    return {"findings": len(after), "groups": len(groups), "false_merges": 0,
            "missed_identical_findings": 0, "lost_paths": 0,
            "backends": dict(collections.Counter(c["backend"] for c in after))}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", type=Path, default=ROOT / "target/portable-release/open-data-recovery")
    p.add_argument("--runs", type=int, default=3)
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args()
    assert args.runs > 0
    args.output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ)
    env["PATH"] = str(ROOT / "target/engine-package/usr/bin") + os.pathsep + env["PATH"]
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    cases = [("unique", "quick"), ("small", "quick"), ("small", "full"),
             ("large", "full"), ("large", "deep"), ("ntfs", "quick"), ("ntfs", "full")]
    report = {"created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "binary_sha256": fixture_tools.sha(args.binary), "runs": args.runs,
              "storage": "RAM-backed /tmp sources and separate /dev/shm workspaces; warm data",
              "fixtures": {}, "results": []}
    with tempfile.TemporaryDirectory(prefix="odr-consolidation-feature-source-", dir="/tmp") as src, \
            tempfile.TemporaryDirectory(prefix="odr-consolidation-feature-work-", dir="/dev/shm") as work:
        source_root, workspace_root = Path(src), Path(work)
        assert source_root.stat().st_dev != workspace_root.stat().st_dev
        specifications = {"unique": (80, 0, 0, 16, "ext2"), "small": (600, 60, 48, 64, "ext2"),
                          "large": (32, 8, 1024, 192, "fat16"), "ntfs": (24, 6, 256, 32, "ntfs")}
        for name, specification in specifications.items():
            print("Creating", name, flush=True)
            report["fixtures"][name] = fixture_tools.create_fixture(source_root, name, *specification)
        for run in range(1, args.runs + 1):
            for name, mode in cases:
                case = f"{name}-{mode}"
                fixture = report["fixtures"][name]
                log = args.output / f"{case}-{run}"
                log.mkdir()
                print(f"Run {run}: {case}", flush=True)
                raw, raw_manifest, off_seconds = scan(args.binary, fixture, mode, workspace_root / "off", False, env, log / "off")
                grouped, manifest, on_seconds = scan(args.binary, fixture, mode, workspace_root / "on", True, env, log / "on")
                assert report.setdefault("engine_versions", manifest["engines"]) == manifest["engines"]
                assert raw_manifest["engines"] == manifest["engines"]
                verification = validate(raw, grouped, fixture, args.binary, env)
                result = {"case": case, "run": run, "off_seconds": off_seconds, "on_seconds": on_seconds,
                          "consolidation_seconds": manifest["consolidation"]["duration_millis"] / 1000,
                          "off_workspace": fixture_tools.footprint(raw), "on_workspace": fixture_tools.footprint(grouped),
                          "consolidation": manifest["consolidation"], "verification": verification}
                report["results"].append(result)
                (log / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
                (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
                print(f"  off={off_seconds:.3f}s on={on_seconds:.3f}s finalization={result['consolidation_seconds']:.3f}s; "
                      f"{verification['findings']} findings -> {verification['groups']} groups", flush=True)
                shutil.rmtree(workspace_root / "off")
                shutil.rmtree(workspace_root / "on")
        for f in report["fixtures"].values():
            f["source_sha256_after"] = fixture_tools.sha(f["image"])
            assert f["source_sha256_before"] == f["source_sha256_after"]
    report["summary"] = {}
    for name, mode in cases:
        case = f"{name}-{mode}"
        rows = [r for r in report["results"] if r["case"] == case]
        report["summary"][case] = {key: {"median": statistics.median(r[key] for r in rows),
                                      "min": min(r[key] for r in rows), "max": max(r[key] for r in rows)}
                                   for key in ("off_seconds", "on_seconds", "consolidation_seconds")}
    (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    print("Passed: exact grouping, preserved raw findings/paths, CLI reopen, and unchanged source hashes.", flush=True)


if __name__ == "__main__":
    main()
