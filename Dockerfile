# syntax=docker/dockerfile:1.2
ARG DOCKER_IO_REGISTRY_PREFIX
FROM --platform=$BUILDPLATFORM ${DOCKER_IO_REGISTRY_PREFIX}rust:1.74 as build

# Install required tools and libc for cross-compilation.
RUN apt update \
    && apt install --yes \
        g++-x86-64-linux-gnu libc6-dev-amd64-cross \
        g++-aarch64-linux-gnu libc6-dev-arm64-cross \
        g++-arm-linux-gnueabihf libc6-dev-armhf-cross \
        g++-i686-linux-gnu libc6-dev-i386-cross \
    && rustup target add x86_64-unknown-linux-gnu \
    && rustup target add aarch64-unknown-linux-gnu \
    && rustup target add armv7-unknown-linux-gnueabihf \
    && rustup target add i686-unknown-linux-gnu

# amd64 libs are at /usr/x86_64-linux-gnu/lib
# but the compilation on arm64 hosts
# expects them to be in /usr/lib/x86_64-linux-gnu
RUN cp -rvt "/usr/lib/x86_64-linux-gnu" $(find /usr/x86_64-linux-gnu/lib) \
    && file /usr/lib/x86_64-linux-gnu/libmvec.a \
    && (ls -lah /usr/lib/x86_64-linux-gnu/libmvec.a || exit 1);

ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    CXX_x86_64_unknown_linux_gnu=x86_64-linux-gnu-g++ \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
    CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
    CC_armv7_unknown_linux_gnueabihf=arm-linux-gnueabihf-gcc \
    CXX_armv7_unknown_linux_gnueabihf=arm-linux-gnueabihf-g++ \
    CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_LINKER=i686-linux-gnu-gcc \
    CC_i686_unknown_linux_gnu=i686-linux-gnu-gcc \
    CXX_i686_unknown_linux_gnu=i686-linux-gnu-g++

ENV RUSTFLAGS="-C target-feature=+crt-static"

WORKDIR /app

COPY [ "Cargo.toml", "Cargo.lock", "./" ]
COPY src src

# download dependencies only once for all target architectures
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo fetch --target 'x86_64-unknown-linux-gnu' \
    && cargo fetch --target 'aarch64-unknown-linux-gnu' \
    && cargo fetch --target 'armv7-unknown-linux-gnueabihf' \
    && cargo fetch --target 'i686-unknown-linux-gnu'

ARG TARGETPLATFORM
ARG TARGETOS
ARG TARGETARCH
ARG TARGETVARIANT

RUN --mount=type=cache,target=/usr/local/cargo/registry,readonly \
    --mount=type=cache,target=/usr/local/cargo/git,readonly \
    --mount=type=cache,target=/app/target \
    printf '\n[profile.release]\n' >> ./Cargo.toml \
    && printf 'lto = "fat"\n' >> ./Cargo.toml \
    && printf 'codegen-units = 1\n' >> ./Cargo.toml \
    && case "${TARGETARCH}/${TARGETVARIANT}" in \
        "amd64/v2") RUSTFLAGS="$(printf '%s %s' "${RUSTFLAGS}" "-C target-cpu=x86-64-v2")";; \
        "amd64/v3") RUSTFLAGS="$(printf '%s %s' "${RUSTFLAGS}" "-C target-cpu=x86-64-v3")";; \
        "amd64/v4") RUSTFLAGS="$(printf '%s %s' "${RUSTFLAGS}" "-C target-cpu=x86-64-v4")";; \
    esac \
    && case "${TARGETARCH}" in \
        "amd64") rust_target="x86_64-unknown-linux-gnu";; \
        "arm64") rust_target="aarch64-unknown-linux-gnu";; \
        "arm") rust_target="armv7-unknown-linux-gnueabihf";; \
        "386") rust_target="i686-unknown-linux-gnu";; \
        *) exit 1;; \
    esac \
    && cargo build --frozen --release --target "${rust_target}" \
    && cp -v "./target/${rust_target}/release/typo3-testing-xml-to-csv" /typo3-testing-xml-to-csv

FROM scratch 
COPY --from=build --chown=0:0 --chmod=500 /typo3-testing-xml-to-csv /
ENTRYPOINT [ "/typo3-testing-xml-to-csv" ]


