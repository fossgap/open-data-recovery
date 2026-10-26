# Hybrid wrapper-first engine

We aggregate existing Linux recovery tools (testdisk/photorec, sleuthkit, ddrescue, foremost/digler) behind a common Scanner/Recoverer interface, replacing hot paths with native code only when proven necessary, to ship complete coverage without rewriting ext4/NTFS undelete logic.
