#!/bin/sh
# Creates a disposable regular image; never opens or writes to a device.
set -eu
cd "$(dirname "$0")/.."
if [ "$#" -gt 1 ]; then
    echo 'Usage: create-restoration-test-image.sh [OUTPUT.img]' >&2
    exit 2
fi
for tool in python3 mkfs.ext2 e2fsck; do
    command -v "$tool" >/dev/null
done
python3 - "${1:-test.img}" <<'PY'
import hashlib
from pathlib import Path
import struct
import subprocess
import sys
import tempfile

output = Path(sys.argv[1])
if output.exists() or output.is_symlink():
    sys.exit(f"Refusing to replace an existing file: {output}")

size = 64 * 1024 * 1024
offset = 1024 * 1024
with tempfile.TemporaryDirectory(prefix="odr-test-image-", dir="/tmp") as temporary:
    root = Path(temporary)
    contents = root / "files"
    contents.mkdir()
    (contents / "README.txt").write_text(
        "Open Data Recovery restoration test image\n\n"
        "This disposable image contains an MBR and one ext2 partition.\n"
        "Use it to review the image restoration wizard and confirmation dialog.\n"
        "Cancel the dialog to finish without writing. Confirming overwrites the selected disk.\n",
        encoding="utf-8",
    )
    (contents / "sample.txt").write_text(
        "ODR test data: this file belongs to a disposable image.\n",
        encoding="utf-8",
    )
    partition = root / "partition.img"
    with partition.open("xb") as file:
        file.truncate(size - offset)
    subprocess.run(
        ["mkfs.ext2", "-q", "-F", "-L", "ODR_TEST", "-d", str(contents), str(partition)],
        check=True,
    )
    subprocess.run(["e2fsck", "-fn", str(partition)], check=True)
    mbr = bytearray(512)
    struct.pack_into("<I", mbr, 440, 0x4F445254)
    # One Linux partition, beginning at sector 2048; use LBA addressing.
    mbr[446:462] = struct.pack(
        "<B3sB3sII", 0, b"\xfe\xff\xff", 0x83, b"\xfe\xff\xff",
        offset // 512, (size - offset) // 512,
    )
    mbr[510:512] = b"\x55\xaa"
    # Exclusive creation also protects against a file appearing during formatting.
    with output.open("xb") as image, partition.open("rb") as source:
        image.truncate(size)
        image.write(mbr)
        image.seek(offset)
        while chunk := source.read(1024 * 1024):
            if any(chunk):
                image.write(chunk)
            else:
                image.seek(len(chunk), 1)

digest = hashlib.sha256()
with output.open("rb") as image:
    while chunk := image.read(1024 * 1024):
        digest.update(chunk)
print(f"Created {output.absolute()} ({size} bytes)")
print(f"SHA-256: {digest.hexdigest()}")
PY
