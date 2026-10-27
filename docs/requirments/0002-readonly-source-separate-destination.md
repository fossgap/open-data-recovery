# Read-only source, separate destination

Source is always opened read-only; recovery writes must go to a separate Recovery Destination, with optional ddrescue clone-first for failing disks. Live means scan/preview on a mounted filesystem without unmount.

Device/folder sources require a different physical drive for both workspace and destination. Regular raw/dd/E01 image sources, including discovered ranges inside images, may share their hosting drive with output folders. Output folders must not contain the image or any segment. Retained clones keep their original device-source policy; image sources retain their exception for recovery/restart/cleanup after disconnection. Path confinement, exclusive output creation, available-space checks and known-storage validation remain mandatory.
