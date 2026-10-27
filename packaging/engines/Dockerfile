FROM golang:1.24-bookworm AS digler
RUN git clone https://github.com/ostafen/digler.git /build && cd /build && git checkout 2f62fc7e90f5c4123e0f0fa9a31c0128bfffc32c
WORKDIR /build
RUN CGO_ENABLED=0 go build -trimpath -ldflags '-s -w -X github.com/ostafen/digler/internal/env.Version=odr-2f62fc7' -o /digler ./cmd

FROM debian:bookworm AS ext4rescue
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates ldc gcc libc6-dev zlib1g-dev python3 && rm -rf /var/lib/apt/lists/*
RUN git clone https://github.com/epi/ext4rescue.git /build && cd /build && git checkout c9002285aec94e17d2e75296a0aabce78183770e
WORKDIR /build
# Cache confinement: respect XDG_CACHE_HOME instead of writing to the user's home.
RUN python3 -c 'from pathlib import Path; p=Path("source/filecache.d"); s=p.read_text().replace("import std.stdio;", "import std.stdio;\nimport std.process : environment;"); s=s.replace("return buildPath(\"~\", \".ext4rescue\", filename).expandTilde();", "return buildPath(environment.get(\"XDG_CACHE_HOME\", expandTilde(\"~\")), \".ext4rescue\", filename);"); p.write_text(s)'
RUN python3 -c 'from pathlib import Path; p=Path("source/extract.d"); s=p.read_text().replace("import std.file : exists;", "import std.file : exists, mkdirRecurse;").replace("_destPath = destPath;", "mkdirRecurse(destPath);\n\t\t_destPath = destPath;"); p.write_text(s)'
RUN ldc2 -O -release -link-defaultlib-shared=false -of=/ext4rescue source/*.d -L--no-as-needed -L-lz
RUN git clone https://github.com/Lazza/RecuperaBit.git /recuperabit && cd /recuperabit && git checkout 90930dc8e104bb2198c1596004d355066d314d4b
RUN mkdir -p /export/usr/bin /export/usr/lib/open-data-recovery-engines /export/usr/share/doc/open-data-recovery-engines/sources && cp /ext4rescue /export/usr/bin/ && cp -a /recuperabit/src/recuperabit /export/usr/lib/open-data-recovery-engines/ && printf '#!/usr/bin/python3\nimport sys\nfrom pathlib import Path\nsys.path.insert(0,str(Path(__file__).resolve().parent.parent / "lib/open-data-recovery-engines"))\nfrom recuperabit.cli import main\nmain()\n' > /export/usr/bin/recuperabit && chmod 755 /export/usr/bin/recuperabit && cp /recuperabit/LICENSE /export/usr/share/doc/open-data-recovery-engines/RecuperaBit.LICENSE && cp /build/COPYING /export/usr/share/doc/open-data-recovery-engines/ext4rescue.COPYING && tar --exclude=.git -czf /export/usr/share/doc/open-data-recovery-engines/sources/ext4rescue.tar.gz -C /build . && tar --exclude=.git -czf /export/usr/share/doc/open-data-recovery-engines/sources/recuperabit.tar.gz -C /recuperabit .
COPY --from=digler /digler /export/usr/bin/digler
COPY --from=digler /build/LICENSE /export/usr/share/doc/open-data-recovery-engines/Digler.LICENSE
CMD ["sh"]
