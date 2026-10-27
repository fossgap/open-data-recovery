# Recovery fixtures

`recovery.png` is a deterministic 128 × 128 RGB test image generated for this
repository: seed `2026`, PNG with uncompressed zlib data. Its size exceeds the
ext2 direct-block range used by the storage test. It contains no user data.

Recreate it from the repository root:

```sh
PYTHONPATH=scripts python3 - <<'PYTHON'
from pathlib import Path
from consolidation_fixtures import png
Path("tests/fixtures/recovery.png").write_bytes(png(128, 128, 2026))
PYTHON
```

Real-engine tests place it inside disposable images and compare recovered bytes.
