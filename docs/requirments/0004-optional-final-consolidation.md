# Optional final consolidation

Approved for implementation after the 2026-10-27 spike.

- Add an unchecked Custom Scan option with a tooltip explaining possible additional scan time and workspace use. The default Scan button keeps consolidation disabled.
- Keep the existing sequential metadata plan and one-carver choice. Consolidation is a separate final stage, not an additional recovery engine.
- Group only verified identical contents. Use length to exclude impossible matches, then complete SHA-256 and byte comparisons. Distinct, truncated and unverified outputs remain separate.
- Publish a stable grouped list when finalization completes. Keep original findings, paths, engine versions and source addresses. Saved scans reopen the same grouping.
- Prefer original metadata names while reusing any verified identical retained content. Preserve original engine outputs; retain new unique extractions and discard only new confirmed duplicate temporary files.
- Select/recover one copy per group by default. Expose all findings and allow recovery of every distinct recorded path, with correct selection counts, destination-space checks and audit records.
- Preserve source protection, pause/cancel handling and bounded paging. Roll back incomplete final grouping on cancellation, insufficient storage or invalidated canonical content. Report individual verification failures explicitly.
- Keep existing scans and default-off scanning compatible. Expose equivalent CLI options.
- Address the per-file completion wait observed in the spike and validate correctness and cost using disposable images before rebuilding local packages.
