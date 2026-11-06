# Builds the wrapper image from source: lite (Android NDK), the web panel and
# the host binary. CI publishes a prebuilt image instead (ghcr.io/itouakirai/
# wrapper, see docker/prebuilt.Dockerfile); use this one to build your own:
#
#   docker build -t ghcr.io/itouakirai/wrapper:latest .
#
# The image runs the native multi-account agent: lite runs chrooted in a user
# namespace inside the container, so the container needs seccomp/apparmor
# relaxed (see compose.yaml) or --privileged. x86_64 (linux/amd64) only: lite
# is an x86_64 Android binary.
ARG NDK_VERSION=23b

FROM debian:13-slim AS lite
ARG NDK_VERSION
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake unzip git curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*
# Only the NDK's LLVM toolchain (compilers + sysroot) is used.
RUN curl -fsSL --retry 3 -o ndk.zip https://dl.google.com/android/repository/android-ndk-r${NDK_VERSION}-linux.zip \
    && unzip -q ndk.zip "android-ndk-r${NDK_VERSION}/toolchains/llvm/*" && rm ndk.zip
COPY CMakeLists.txt import.h ./
COPY lite ./lite
COPY rootfs ./rootfs
RUN cmake -S . -B build -DCMAKE_BUILD_TYPE=Release -DANDROID_NDK_PATH=/app/android-ndk-r${NDK_VERSION} \
    && cmake --build build -j"$(nproc)"

FROM node:24-trixie-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN npm run build

FROM rust:1-trixie AS wrapper
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY --from=web /web/dist ./web/dist
RUN cargo build --release -p wrapper

FROM debian:13-slim
WORKDIR /app
COPY --from=lite /app/rootfs /app/rootfs
COPY --from=wrapper /src/target/release/wrapper /app/wrapper
# Account data (logins, sessions, statistics, settings): mount it.
VOLUME /app/rootfs/data
EXPOSE 12340
# The web panel and the API share port 12340. WRAPPER_API_TOKEN protects
# /login, /logout and /api (and signs in to the panel); WRAPPER_PUBLIC=true
# turns public mode on, WRAPPER_MAX_ACCOUNTS caps it. See README.md.
ENV WRAPPER_HOST=0.0.0.0 WRAPPER_PORT=12340
CMD ["/app/wrapper", "agent"]
