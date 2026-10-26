# Backend matrix per scan mode

Quick (metadata undelete, keeps names): ext4magic + extundelete (ext3/4 journal), ntfsundelete + RecuperaBit (NTFS MFT), TSK fls/icat/tsk_recover + testdisk, btrfs restore / btrfs-find-root native, debugfs/dumpe2fs for ext forensics, ext4rescue on ddrescue image+log.
Deep (carve, content-based): photorec + scalpel (preferred) + foremost/digler + magicrescue/recoverjpeg/bulk_extractor via same carver trait.
Clone-first (safety pre-step for failing media): ddrescue + safecopy. E01 via ewf-tools.
Full Scan (Quick + Deep): For maximum recovery chance 
All backends sit behind a common Scanner/Recoverer trait with versions recorded in the snapshot. All are packaged in Debian/Fedora/Arch so native deps stay clean.
