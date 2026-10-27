#!/usr/bin/env python3
"""Reproducible, synthetic, regular-file-only consolidation benchmark.

Creates images in /tmp and scan workspaces on a separate tmpfs (/dev/shm).
No mounts, real devices, production edits, or source writes during scans.
"""
import argparse
import collections
import ctypes
import ctypes.util
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import shutil
import sqlite3
import statistics
import struct
import subprocess
import tempfile
import time
import zlib

ROOT = Path(__file__).resolve().parents[2]
MIB = 1024 * 1024


def command(args, **kwargs):
    return subprocess.run([str(x) for x in args], check=True, capture_output=True,
                          text=True, **kwargs)


def sha(path):
    with open(path, "rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def png(width, height, seed):
    rng = random.Random(seed)
    raw = b"".join(b"\0" + rng.randbytes(width * 3) for _ in range(height))

    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data)))

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 0)) + chunk(b"IEND", b""))


def footprint(root):
    """Sum logical and st_blocks bytes, counting hardlinks only once."""
    logical = allocated = 0
    seen = set()
    for p in [root, *root.rglob("*")]:
        s = p.lstat()
        key = (s.st_dev, s.st_ino)
        if key in seen:
            continue
        seen.add(key)
        if p.is_file():
            logical += s.st_size
        allocated += s.st_blocks * 512
    return {"logical_bytes": logical, "allocated_bytes": allocated}


def artifact_breakdown(session):
    counts = collections.Counter()
    sizes = collections.Counter()
    for p in (session / "artifacts").rglob("*"):
        if p.is_file():
            key = p.suffix or "(no suffix)"
            counts[key] += 1
            sizes[key] += p.stat().st_size
    return {key: {"files": counts[key], "logical_bytes": sizes[key]}
            for key in sorted(counts)}


def unlink_ntfs_fixture(image, files):
    """Use libntfs-3g to unlink files INSIDE our new image; no kernel/FUSE mount.

    ABI: tuxera/ntfs-3g 2022.10.3 include/ntfs-3g/{dir,volume}.h.
    ntfs_delete closes both the child and parent inode handles.
    """
    assert image.is_file() and image.name == "source.img"
    lib = ctypes.CDLL(ctypes.util.find_library("ntfs-3g"), use_errno=True)
    ptr = ctypes.c_void_p
    lib.ntfs_mount.argtypes = [ctypes.c_char_p, ctypes.c_ulong]
    lib.ntfs_mount.restype = ptr
    lib.ntfs_pathname_to_inode.argtypes = [ptr, ptr, ctypes.c_char_p]
    lib.ntfs_pathname_to_inode.restype = ptr
    lib.ntfs_delete.argtypes = [ptr, ctypes.c_char_p, ptr, ptr, ptr, ctypes.c_uint8]
    lib.ntfs_delete.restype = ctypes.c_int
    lib.ntfs_umount.argtypes = [ptr, ctypes.c_int]
    lib.ntfs_umount.restype = ctypes.c_int
    volume = lib.ntfs_mount(os.fsencode(image), 0)
    if not volume:
        raise OSError(ctypes.get_errno(), "Opening NTFS fixture")
    try:
        for f in files:
            name = f["name"]
            child = lib.ntfs_pathname_to_inode(volume, None, ("/" + name).encode())
            parent = lib.ntfs_pathname_to_inode(volume, None, b"/")
            assert parent and child
            unicode_name = ctypes.create_string_buffer(name.encode("utf-16-le"))
            if lib.ntfs_delete(volume, ("/" + name).encode(), child, parent,
                               unicode_name, len(name)):
                raise OSError(ctypes.get_errno(), "Unlinking NTFS fixture file")
    finally:
        if lib.ntfs_umount(volume, 0):
            raise OSError(ctypes.get_errno(), "Closing NTFS fixture")


def create_fixture(parent, name, count, copies, width, image_mib, filesystem="ext2"):
    root = parent / name
    root.mkdir()
    payloads = root / "payloads"
    payloads.mkdir()
    files = []
    for index in range(count + copies):
        if index < count:
            data = (png(width, width, 1000 + index) if width else
                    random.Random(1000 + index).randbytes(4096 + index * 37))
        else:
            data = (payloads / files[index - count]["name"]).read_bytes()
        filename = f"file-{index:04d}.{'png' if width else 'bin'}"
        (payloads / filename).write_bytes(data)
        files.append({"name": filename, "bytes": len(data),
                      "sha256": hashlib.sha256(data).hexdigest()})
    image = root / "source.img"
    with image.open("xb") as f:
        f.truncate(image_mib * MIB)
    if filesystem == "ext2":
        command(["mkfs.ext2", "-q", "-F", "-b", "4096", image])
        # Allocate ALL files first, then unlink all. No reuse of freed blocks.
        commands = [f"write {payloads / f['name']} /{f['name']}" for f in files]
        commands += [f"rm /{f['name']}" for f in files]
        script = root / "debugfs.commands"
        script.write_text("\n".join(commands) + "\n")
        result = command(["debugfs", "-w", "-f", script, image])
        (root / "fixture.log").write_text(result.stdout + result.stderr)
        assert result.stdout.count("Allocated inode:") == len(files), result.stderr
    elif filesystem == "fat16":
        command(["mkfs.fat", "-F", "16", image])
        for f in files:
            command(["mcopy", "-i", image, payloads / f["name"], "::" + f["name"]])
        command(["mdel", "-i", image, "::*"])
    else:
        command(["mkfs.ntfs", "-F", "-Q", "-q", image])
        for f in files:
            command(["ntfscp", image, payloads / f["name"], "/" + f["name"]])
        unlink_ntfs_fixture(image, files)
    # Do not keep a second payload copy in RAM throughout the benchmark.
    shutil.rmtree(payloads)
    image.chmod(0o400)
    return {"name": name, "image": str(image), "filesystem": filesystem,
            "deleted": True, "image_bytes": image.stat().st_size,
            "files": files, "file_count": len(files), "unique_contents": count,
            "payload_bytes": sum(f["bytes"] for f in files),
            "source_sha256_before": sha(image), "extension": "png" if width else "bin"}


def readonly_db(path):
    db = sqlite3.connect(path.as_uri() + "?mode=ro&immutable=1", uri=True)
    db.row_factory = sqlite3.Row
    return db


def verify(session, output, fixture, result):
    """Independent post-timing content oracle: reject false merges or lost paths."""
    with readonly_db(session / "tree.sqlite") as db:
        candidates = [dict(r) for r in db.execute("SELECT * FROM candidates ORDER BY id")]
    with readonly_db(output / "groups.sqlite") as db:
        membership = {r["candidate_id"]: r["group_id"] for r in db.execute("SELECT * FROM members")}
        digests = {r["id"]: r["sha256"] for r in db.execute("SELECT * FROM groups")}
        content_paths = {r["id"]: r["content"] for r in db.execute("SELECT * FROM groups")} \
            if "content" in [r["name"] for r in db.execute("PRAGMA table_info(groups)")] else {}
        assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert set(membership) == {c["id"] for c in candidates}
    expected = {f["name"]: f for f in fixture["files"]}
    content_to_group = {}
    group_to_content = {}
    backends = collections.Counter()
    known_findings = 0
    original_paths = set()
    for c in candidates:
        backends[c["backend"]] += 1
        if c["artifact"]:
            content = session / c["artifact"]
        else:
            content = output / "cache" / f"{c['id']}.bin"
        if content.exists():
            digest = sha(content)
            assert content.stat().st_size == c["bytes"]
        else:
            # Unique-size TSK rows are intentionally not materialized by the spike.
            # Verify these independently with icat after measured work has ended.
            manifest = json.loads((session / "manifest.json").read_text())
            recovered = subprocess.run([
                "icat", "-r", "-b", str(manifest["source"]["sector_size"]),
                "-o", str(c["offset_bytes"] // manifest["source"]["sector_size"]),
                fixture["image"], c["locator"]], check=True, capture_output=True).stdout
            assert len(recovered) == c["bytes"]
            digest = hashlib.sha256(recovered).hexdigest()
        group = membership[c["id"]]
        assert content_to_group.setdefault(digest, group) == group, "Missed exact duplicate"
        assert group_to_content.setdefault(group, digest) == digest, "False merge"
        if digests[group] is not None:
            assert digests[group] == digest
        filename = Path(c["path"]).name
        if filename in expected:
            assert digest == expected[filename]["sha256"], "Wrong reconstructed file"
            known_findings += 1
        if c["backend"] == "tsk":
            original_paths.add(filename)
    for group, content in content_paths.items():
        if content is not None:
            assert sha(Path(content)) == group_to_content[group], "Invalid retained canonical content"
    assert len(group_to_content) == result["groups"]
    # Engines can miss deleted directory entries. Verify consolidation against ALL
    # actual findings, and report source-content coverage separately from grouping.
    expected_hashes = {f["sha256"] for f in fixture["files"]}
    return {"backends": dict(backends), "verified_findings": len(candidates),
            "verified_named_findings": known_findings, "original_paths": len(original_paths),
            "fixture_unique_contents_found": len(set(content_to_group) & expected_hashes),
            "fixture_unique_contents_missing": len(expected_hashes - set(content_to_group)),
            "contents_not_in_fixture": len(set(content_to_group) - expected_hashes),
            "false_merges": 0, "missed_identical_findings": 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--cache-policy", choices=["all", "reuse"], default="all")
    parser.add_argument("--cases", default="unique-quick,small-quick,small-full,large-full,large-deep,ntfs-quick,ntfs-full")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--workspace-root", type=Path, default=Path("/dev/shm"))
    parser.add_argument("--binary", type=Path, default=ROOT / "target/portable-release/open-data-recovery")
    parser.add_argument("--prototype", type=Path, default=ROOT / "target/consolidation-spike-build/release/odr-consolidation-spike")
    args = parser.parse_args()
    assert args.runs >= 1
    args.output.mkdir(parents=True, exist_ok=False)
    cases = {
        "unique-quick": ("unique", "quick"), "small-quick": ("small", "quick"),
        "small-full": ("small", "full"), "large-full": ("large", "full"),
        "large-deep": ("large", "deep"), "ntfs-quick": ("ntfs", "quick"),
        "ntfs-full": ("ntfs", "full"),
    }
    selected = args.cases.split(",")
    assert all(case in cases for case in selected)
    env = dict(os.environ)
    env["PATH"] = str(ROOT / "target/engine-package/usr/bin") + os.pathsep + env["PATH"]
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    report = {"created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "commit": command(["git", "rev-parse", "HEAD"], cwd=ROOT).stdout.strip(),
              "app_binary_sha256": sha(args.binary), "prototype_sha256": sha(args.prototype),
              "platform": platform.platform(), "python": platform.python_version(),
              "cpu": command(["lscpu"]).stdout,
              "storage": command(["df", "-T", "/tmp", args.workspace_root]).stdout,
              "runs_per_case": args.runs, "fixtures": {}, "results": [],
              "cache_policy": args.cache_policy,
              "policy": "Preserve existing artifacts; SHA-256 plus byte comparison; cache policy=" + args.cache_policy,
              "engines": command([args.binary, "--backends"], env=env).stdout}
    with tempfile.TemporaryDirectory(prefix="odr-consolidation-source-", dir="/tmp") as source_tmp, \
            tempfile.TemporaryDirectory(prefix="odr-consolidation-work-", dir=args.workspace_root) as work_tmp:
        source_root, work_root = Path(source_tmp), Path(work_tmp)
        assert source_root.stat().st_dev != work_root.stat().st_dev
        specs = {
            "unique": (80, 0, 0, 16, "ext2"),
            "small": (600, 60, 48, 64, "ext2"),
            "large": (32, 8, 1024, 192, "fat16"),
            "ntfs": (24, 6, 256, 32, "ntfs"),
        }
        for name in dict.fromkeys(cases[c][0] for c in selected):
            print(f"Creating {name} fixture", flush=True)
            report["fixtures"][name] = create_fixture(source_root, name, *specs[name])
        # Round-robin cases to reduce sustained load/order bias.
        for run in range(1, args.runs + 1):
            for case in selected:
                name, mode = cases[case]
                fixture = report["fixtures"][name]
                workspace = work_root / f"{case}-{run}"
                cli = [args.binary, "--scan", fixture["image"], "--workspace", workspace,
                       "--mode", mode, "--extensions", fixture["extension"]]
                print(f"Run {run}: {case}: current scan", flush=True)
                started = time.perf_counter()
                scan = subprocess.run([str(x) for x in cli], capture_output=True, text=True,
                                      env=env, timeout=600)
                scan_seconds = time.perf_counter() - started
                log = args.output / f"{case}-{run}"
                log.mkdir()
                (log / "scan.stdout").write_text(scan.stdout)
                (log / "scan.stderr").write_text(scan.stderr)
                assert scan.returncode == 0, f"Scan failed: {scan.stderr}"
                session = Path(next(line[6:] for line in scan.stdout.splitlines() if line.startswith("SAVED=")))
                manifest = json.loads((session / "manifest.json").read_text())
                assert manifest["complete"] and not manifest["errors"]
                before = footprint(session)
                original_catalog_hash = sha(session / "tree.sqlite")
                original_manifest_hash = sha(session / "manifest.json")
                output = workspace / "consolidation-spike"
                print(f"Run {run}: {case}: final consolidation", flush=True)
                started = time.perf_counter()
                consolidated = command([args.prototype, session, output, args.cache_policy], env=env, timeout=600)
                final_seconds = time.perf_counter() - started
                stage = json.loads(consolidated.stdout)
                added = footprint(output)
                validation = verify(session, output, fixture, stage)
                assert original_catalog_hash == sha(session / "tree.sqlite")
                assert original_manifest_hash == sha(session / "manifest.json")
                assert before == footprint(session), "Original workspace was changed"
                result = {"case": case, "run": run, "scan_seconds": scan_seconds,
                          "finalization_seconds": final_seconds,
                          "total_seconds": scan_seconds + final_seconds,
                          "baseline_workspace": before, "added_workspace": added,
                          "baseline_artifacts": artifact_breakdown(session),
                          "consolidation": stage, "verification": validation,
                          "command": [str(x) for x in cli]}
                report["results"].append(result)
                shutil.copyfile(session / "manifest.json", log / "scan-manifest.json")
                shutil.copyfile(session / "tree.sqlite", log / "tree.sqlite")
                shutil.copyfile(output / "groups.sqlite", log / "groups.sqlite")
                (log / "result.json").write_text(json.dumps(result, indent=2) + "\n")
                (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
                print(f"  scan={scan_seconds:.3f}s + consolidate={final_seconds:.3f}s; "
                      f"{stage['candidates']} -> {stage['groups']}; "
                      f"workspace={before['allocated_bytes']/MIB:.3f} + {added['allocated_bytes']/MIB:.3f} MiB", flush=True)
                shutil.rmtree(workspace)
        for fixture in report["fixtures"].values():
            fixture["source_sha256_after"] = sha(fixture["image"])
            assert fixture["source_sha256_before"] == fixture["source_sha256_after"]
    report["summary"] = {}
    for case in selected:
        rows = [r for r in report["results"] if r["case"] == case]
        summary = {}
        for metric in ("scan_seconds", "finalization_seconds", "total_seconds"):
            values = [r[metric] for r in rows]
            summary[metric] = {"median": statistics.median(values), "min": min(values), "max": max(values)}
        report["summary"][case] = summary
    (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    print("Completed; all source hashes unchanged and all content groups verified.", flush=True)


if __name__ == "__main__":
    main()
