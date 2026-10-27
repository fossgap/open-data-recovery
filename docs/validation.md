# Implementation validation

Validated on 2026-10-26. The release binary was built against Debian Bookworm's GTK 4.8 and glibc 2.36. Integration checks used regular disposable image files and separate temporary filesystems; no physical recovery source was scanned or formatted.

| Check | Result |
| --- | --- |
| Rust tests | 13 passed; 4 integration tests run separately |
| Disposable-image integration tests | All 4 passed |
| GtkBuilder layouts | All validated |
| Demo and production GTK smoke checks | Passed with the release binary |
| Production GUI recovery | Passed with the development binary and packaged AppImage |
| Release packaging | Debian, RPM, Arch and AppImage artifacts built locally |

Integration cases cover deleted ext2 content via TSK, ddrescue cloning, reopening and rescanning after disconnecting the original source, byte-exact recovery, PhotoRec/Scalpel/Foremost/Digler/Magicrescue carving, compound NTFS metadata addresses, RecuperaBit reconstruction, E01 normalization, and ext4rescue with a retained clone and map. Recovery checks compare known bytes and recorded SHA-256 verification.

The GUI recovery check opens a saved scan, selects a candidate, validates a separate destination, confirms recovery options, runs recovery, and checks the resulting file and audit record. The AppImage also verifies interaction with host recovery tools while using its bundled GTK runtime.

These checks validate specific fixtures and workflows. They do not establish recovery success for every damaged filesystem or installed backend version, and native packages have not been installed into the host system.

Reproduce with `scripts/check.sh`, `scripts/check-engines.sh`, and `scripts/check-ui-recovery.sh`. Optional integration engines must be installed or present on `PATH`; the complete run included the locally packaged RecuperaBit, Digler and ext4rescue builds. See [packaging](packaging.md) for testing a packaged binary.
