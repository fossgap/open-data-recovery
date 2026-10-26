# Snapshot storage: SQLite, JSON, artifacts, and optional clone

The default root is `$XDG_DATA_HOME/open-data-recovery/snapshots`, falling back to
`~/.local/share/open-data-recovery/snapshots`. The user may choose a workspace on
another drive. Storage is initialized only after checking it against the source
and its device ancestry.

Each session contains `manifest.json`, `tree.sqlite`, `events.log`, `source.json`,
and copied artifacts. Clone-first retains `clone.img` and its map.  Reopening a saved clone does not require the original source.

Block fingerprints include partition UUID/offset and filesystem UUID as well as
drive identity and size. Snapshot comparison uses the engine versions recorded
with that attempt. NTFS compound addresses and partition byte ranges persist in
the tree; legacy trees remain readable without migration writes during loading.
Recovery appends output paths, errors, and SHA-256 records to the session log.
