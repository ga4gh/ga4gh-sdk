# OCI image of the ga4gh-cli client (see docs/container-image.md). It builds
# the CLI from the checked-out sources with the workspace lockfile and runs it
# as a non-root user. It holds no configuration, credentials or task data:
# callers mount them at run time.
#
# Build from the repository root: docker build -t ga4gh-cli .
#
# The bases are pinned by digest. The Rust toolchain comes from
# rust-toolchain.toml; keep the builder's Rust version in step with it, or the
# build downloads that toolchain first.
ARG RUST_IMAGE=docker.io/library/rust:1.97.0-slim-trixie@sha256:686a437ead83701e8f871e66e838c3ec55f46b5fc235b025756396ac823bdc51
ARG RUNTIME_IMAGE=docker.io/library/debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2

FROM ${RUST_IMAGE} AS builder
RUN apt-get update \
    && apt-get install --yes --no-install-recommends libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --locked --package ga4gh-cli \
    && install -m 0755 target/release/ga4gh-cli /usr/local/bin/ga4gh-cli

FROM ${RUNTIME_IMAGE}
# OpenSSL and the CA certificates for HTTPS to GA4GH services
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates libssl3t64 \
    && rm -rf /var/lib/apt/lists/*
# The CLI reads $HOME/.ga4gh-cli/config.json. HOME is set explicitly, so the
# path stays the same when a platform runs the image with another UID.
RUN groupadd --gid 10001 ga4gh \
    && useradd --uid 10001 --gid 10001 --home-dir /home/ga4gh --create-home \
        --shell /usr/sbin/nologin ga4gh \
    && install -d -o 10001 -g 10001 -m 0755 /work
COPY --from=builder /usr/local/bin/ga4gh-cli /usr/local/bin/ga4gh-cli
ENV HOME=/home/ga4gh
WORKDIR /work
USER 10001:10001
ENTRYPOINT ["ga4gh-cli"]
CMD ["--help"]
