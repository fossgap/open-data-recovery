# Complete filesystem scope

Complete means ext2/3/4 + btrfs + FAT12/16/32 + exFAT + NTFS + XFS carve best-effort. APFS/HFS read-only carve only, no ZFS, decrypt-then-recover only via already-unlocked mapper. No phasing: all in scope for design and implementation.
