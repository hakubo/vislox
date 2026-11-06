# vislox in a container: the viewer, git, and the lox CLI it runs for Pull,
# Sync and Apply. See docker/README.md for how to run it.

FROM rust:1-bookworm AS build
WORKDIR /src
# Dependencies first, so a source change does not rebuild them.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src
COPY build.rs ./
COPY src src
COPY web web
COPY assets assets
# The commit, for `vislox --version` and the UI (the build has no .git).
ARG VISLOX_COMMIT=
RUN touch src/main.rs && VISLOX_COMMIT="$VISLOX_COMMIT" cargo build --release --locked

FROM debian:bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/hakubo/vislox" \
      org.opencontainers.image.description="A Loxone Config-style view of Loxone configs managed by lox, with history, change review and live values" \
      org.opencontainers.image.licenses="Apache-2.0"
ARG TARGETARCH
# The lox release vislox runs (static binaries from github.com/discostu105/lox).
ARG LOX_VERSION=v0.83.0
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git openssh-client tini \
    && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    case "$TARGETARCH" in amd64) a=x86_64 ;; arm64) a=aarch64 ;; *) echo "unsupported: $TARGETARCH" >&2; exit 1 ;; esac; \
    base="https://github.com/discostu105/lox/releases/download/$LOX_VERSION"; \
    curl -fsSLo /usr/local/bin/lox "$base/lox-linux-$a"; \
    echo "$(curl -fsSL "$base/lox-linux-$a.sha256" | cut -d' ' -f1)  /usr/local/bin/lox" | sha256sum -c -; \
    chmod +x /usr/local/bin/lox
COPY --from=build /src/target/release/vislox /usr/local/bin/vislox
COPY docker/entrypoint.sh /usr/local/bin/vislox-entrypoint
RUN useradd --create-home --uid 1000 vislox && mkdir /data && chown vislox /data
USER vislox
WORKDIR /data
VOLUME /data
EXPOSE 7878
ENTRYPOINT ["tini", "--", "vislox-entrypoint"]
