"""Deterministic disposable-image fixtures for consolidation validation.

Only newly created regular image files are formatted or modified. This module
has no command-line entry point and performs no work when imported.
"""
import ctypes
import ctypes.util
import hashlib
import os
import random
import shutil
import sqlite3
import struct
import subprocess
import zlib

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
