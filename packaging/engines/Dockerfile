FROM golang:1.24-bookworm AS digler
RUN git clone https://github.com/ostafen/digler.git /build && cd /build && git checkout 2f62fc7e90f5c4123e0f0fa9a31c0128bfffc32c
WORKDIR /build
RUN CGO_ENABLED=0 go build -trimpath -ldflags '-s -w -X github.com/ostafen/digler/internal/env.Version=odr-2f62fc7' -o /digler ./cmd

FROM debian:bookworm AS engines
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates && rm -rf /var/lib/apt/lists/*
RUN git clone https://github.com/Lazza/RecuperaBit.git /recuperabit && cd /recuperabit && git checkout 90930dc8e104bb2198c1596004d355066d314d4b
RUN mkdir -p /export/usr/bin /export/usr/lib/open-data-recovery-engines /export/usr/share/doc/open-data-recovery-engines/sources && cp -a /recuperabit/src/recuperabit /export/usr/lib/open-data-recovery-engines/ && printf '#!/usr/bin/python3\nimport sys\nfrom pathlib import Path\nsys.path.insert(0,str(Path(__file__).resolve().parent.parent / "lib/open-data-recovery-engines"))\nfrom recuperabit.cli import main\nmain()\n' > /export/usr/bin/recuperabit && chmod 755 /export/usr/bin/recuperabit && cp /recuperabit/LICENSE /export/usr/share/doc/open-data-recovery-engines/RecuperaBit.LICENSE && tar --exclude=.git -czf /export/usr/share/doc/open-data-recovery-engines/sources/recuperabit.tar.gz -C /recuperabit .
COPY --from=digler /digler /export/usr/bin/digler
COPY --from=digler /build/LICENSE /export/usr/share/doc/open-data-recovery-engines/Digler.LICENSE
CMD ["sh"]
