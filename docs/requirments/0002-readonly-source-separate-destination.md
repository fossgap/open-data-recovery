# Read-only source, separate destination

Source is always opened read-only; recovery writes must go to a separate Recovery Destination, with optional ddrescue clone-first for failing disks. Live means scan/preview on a mounted filesystem without unmount.
