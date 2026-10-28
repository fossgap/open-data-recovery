#!/bin/sh
# Catalog checks use Python's standard library; GTK checks run in a private display.
set -eu
cd "$(dirname "$0")/.."
python3 - <<'PY'
import json
import re
from pathlib import Path
import xml.etree.ElementTree as ET

root = Path('src/i18n')
english = json.loads((root / 'en.json').read_text())
required = set()
for path in Path('ui').glob('*.ui'):
    tree = ET.parse(path)
    required.update(node.text for node in tree.iter() if node.get('translatable') == 'yes' and node.text)
for path in Path('src').rglob('*.rs'):
    if path.name.endswith('tests.rs'):
        continue
    for match in re.finditer(r'\b(?:tr\(|trf!\()\s*("(?:[^"\\]|\\.)*")', path.read_text()):
        required.add(json.loads(match[1]))
assert required <= english.keys(), f'Missing English messages: {sorted(required - english.keys())}'
for code in ('fr', 'es', 'zh_CN', 'hi', 'ar', 'pt'):
    catalog = json.loads((root / f'{code}.json').read_text())
    assert catalog.keys() == english.keys(), f'{code}: catalog keys differ'
    for message, translation in catalog.items():
        assert translation.strip(), f'{code}: empty translation: {message}'
        fields = lambda s: sorted(re.findall(r'\{([^{}]+)\}', s))
        assert fields(message) == fields(translation), f'{code}: placeholder mismatch: {message}'
print(f'Catalog coverage: {len(english)} messages in all six translations')
PY
if [ "${1:-}" = --catalogs-only ]; then exit 0; fi
if [ "${1:-}" != --inside ]; then
    exec env GTK_IM_MODULE=simple GDK_BACKEND=x11 GSK_RENDERER=cairo \
        xvfb-run -a -s '-screen 0 1280x900x24' sh scripts/check-i18n.sh --inside
fi
: "${ODR_BINARY:=target/debug/open-data-recovery}"
fixture=$(mktemp -d "${TMPDIR:-/tmp}/odr-i18n-XXXXXX")
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
for language in en fr es zh_CN hi ar pt; do
    XDG_CONFIG_HOME="$fixture" ODR_LANGUAGE="$language" G_DEBUG=fatal-criticals \
        "$ODR_BINARY" --i18n-smoke-test
done
