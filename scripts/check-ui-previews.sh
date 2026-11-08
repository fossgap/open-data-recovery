#!/bin/sh
# Decoder and production GTK checks on a disposable ext2 image only.
set -eu
cd "$(dirname "$0")/.."
# Exercise the production decoder's CPU/time limits with optimized code.
if [ -z "${ODR_BINARY:-}" ]; then cargo build --release --locked; fi
binary=$(realpath "${ODR_BINARY:-${CARGO_TARGET_DIR:-target}/release/open-data-recovery}")
root=$(mktemp -d /tmp/odr-preview-check.XXXXXX)
trap 'rm -rf "$root"' EXIT HUP INT TERM
python3 - "$root" "$binary" <<'PY'
import pathlib, struct, subprocess, sys, zlib
root, binary = pathlib.Path(sys.argv[1]), sys.argv[2]
def chunk(kind, data):
    return struct.pack('!I', len(data)) + kind + data + struct.pack('!I', zlib.crc32(kind + data))
def png(width, height, data):
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('!2I5B', width, height, 8, 6, 0, 0, 0)) + chunk(b'IDAT', data) + chunk(b'IEND', b'')
compressor = zlib.compressobj()
row = b'\0' + b'\x40\x80\xc0\xff' * 8192
data = b''.join(compressor.compress(row) for _ in range(8192)) + compressor.flush()
(root/'large.png').write_bytes(png(8192, 8192, data))
(root/'broken.png').write_bytes(b'not a valid PNG')
(root/'excessive.png').write_bytes(png(50000, 50000, zlib.compress(b'\0\0\0\0\xff')))
(root/'message.txt').write_text('Preview worker stays responsive.\n')
for name in ('large.png', 'broken.png', 'excessive.png'):
    with (root/name).open('rb') as image:
        result = subprocess.run([binary, '--decode-preview'], stdin=image, capture_output=True, timeout=10)
    if name == 'large.png':
        assert result.returncode == 0, result.stderr
        assert result.stdout[:12] == b'ODRP' + struct.pack('<2I', 1024, 1024)
        assert len(result.stdout) == 12 + 1024 * 1024 * 4
        assert result.stdout[12:] == b'\x40\x80\xc0\xff' * (1024 * 1024)
    else:
        assert result.returncode != 0 and not result.stdout, (name, result.stderr)
print('Isolated decoder: exact thumbnail pixels, malformed input and excessive dimensions passed.')
PY
truncate -s 16M "$root/source.img"
mkfs.ext2 -q -F "$root/source.img"
for name in large.png broken.png excessive.png message.txt; do
    debugfs -w -R "write $root/$name $name" "$root/source.img" >/dev/null 2>&1
done
source_hash=$(sha256sum "$root/source.img" | cut -d ' ' -f 1)
"$binary" --scan "$root/source.img" --workspace "$root/scans" --mode quick --metadata tsk --include-existing --extensions png,txt > "$root/scan.log"
session=$(sed -n 's/^SAVED=//p' "$root/scan.log")
test -n "$session"
env ODR_SMOKE_PREVIEWS="$session" ODR_LANGUAGE=en GTK_IM_MODULE=simple GTK_A11Y=none \
    G_DEBUG=fatal-criticals GDK_BACKEND=x11 GSK_RENDERER=cairo \
    xvfb-run -a "$binary" --live-smoke-test
"$binary" --recover "$session" --destination "$root/recovered" --all > "$root/recovery.log"
for name in large.png broken.png excessive.png message.txt; do
    cmp "$root/$name" "$root/recovered/$name"
done
test "$source_hash" = "$(sha256sum "$root/source.img" | cut -d ' ' -f 1)"
test -z "$(find "$session/artifacts/previews" -type f -name '*.png' -size 0)"
printf '%s\n' 'All four originals recovered byte-for-byte after preview errors; source image unchanged.'
