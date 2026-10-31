"""Exercise the installed engine bundle through ODR on disposable images."""
import hashlib
from pathlib import Path
import subprocess
import sys

binary, directory = sys.argv[1:]
root = Path(directory)
payload = Path("/workspace/tests/fixtures/recovery.png").read_bytes()
raw = root / "carve.img"
raw.write_bytes(bytes(1024 * 1024) + payload + bytes(1024 * 1024))


def scan(source, name, options):
    before = hashlib.sha256(source.read_bytes()).hexdigest()
    result = subprocess.run(
        [binary, "--scan", str(source), "--workspace", str(root / name), *options],
        capture_output=True, text=True, timeout=90,
    )
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    result.check_returncode()
    sessions = [line.removeprefix("SAVED=") for line in result.stdout.splitlines()
                if line.startswith("SAVED=")]
    assert len(sessions) == 1, "Expected one saved scan"
    destination = root / (name + "-recovered")
    subprocess.run(
        [binary, "--recover", sessions[0], "--all", "--destination", str(destination)],
        check=True, timeout=90,
    )
    assert hashlib.sha256(source.read_bytes()).hexdigest() == before
    return [path.read_bytes() for path in destination.rglob("*") if path.is_file()]


assert payload in scan(
    raw, "digler", ["--mode", "deep", "--carver", "digler", "--extensions", "png"]
)
print("Installed Digler recovered the embedded PNG.")

ntfs = root / "ntfs.img"
with ntfs.open("wb") as image:
    image.truncate(32 * 1024 * 1024)
subprocess.run(["mkfs.ntfs", "-F", "-Q", "-q", str(ntfs)], check=True)
original = root / "original.txt"
original.write_text("Installed RecuperaBit reconstruction test\n")
subprocess.run(["ntfscp", str(ntfs), str(original), "/recovered.txt"], check=True)
assert original.read_bytes() in scan(
    ntfs, "recuperabit",
    ["--mode", "quick", "--metadata", "recuperabit", "--include-existing", "--extensions", "txt"],
)
print("Installed RecuperaBit reconstructed and recovered the NTFS file.")
