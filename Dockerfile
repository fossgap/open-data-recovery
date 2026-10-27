FROM rust:1.90-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       build-essential pkg-config libgtk-4-dev libgtk-4-bin \
       xvfb xauth dbus-x11 xdotool imagemagick \
       sleuthkit testdisk gddrescue ext4magic extundelete ntfs-3g btrfs-progs \
       e2fsprogs ewf-tools scalpel foremost safecopy magicrescue recoverjpeg \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY ui ./ui
COPY scripts ./scripts
COPY tests ./tests
COPY packaging ./packaging
# Source/storage checks deliberately reject overlay filesystems.
RUN --mount=type=tmpfs,target=/tmp cargo build --locked && cargo test --locked
COPY README.md LICENSE ./

ENV GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_IM_MODULE=simple
CMD ["sh", "scripts/check.sh"]
