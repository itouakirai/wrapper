# Native multi-account agent: lite runs chrooted in a user namespace inside
# the container, so the container needs seccomp/apparmor relaxed (see
# compose.yaml) or --privileged.
ARG NDK_VERSION=23

FROM debian:13.2 AS lite
ARG NDK_VERSION
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake unzip git ca-certificates aria2 \
    && rm -rf /var/lib/apt/lists/*
RUN aria2c -o ndk.zip https://dl.google.com/android/repository/android-ndk-r${NDK_VERSION}b-linux.zip \
    && unzip -q ndk.zip && rm ndk.zip
COPY CMakeLists.txt import.h ./
COPY lite ./lite
COPY rootfs ./rootfs
RUN cmake -S . -B build -DCMAKE_BUILD_TYPE=Release && cmake --build build -j"$(nproc)"

FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN npm run build

FROM rust:1.90-bookworm AS wrapper
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY --from=web /web/dist ./web/dist
RUN cargo build --release -p wrapper

FROM debian:13.2
WORKDIR /app
COPY --from=lite /app/rootfs /app/rootfs
COPY --from=wrapper /src/target/release/wrapper /app/wrapper
EXPOSE 12340
# The web panel and the API share port 12340. Set WRAPPER_API_TOKEN to protect
# /login, /logout and /api (and to sign in to the panel); WRAPPER_PUBLIC=true
# turns public mode on, WRAPPER_MAX_ACCOUNTS caps it.
CMD ["/app/wrapper", "agent", "--host", "0.0.0.0", "--port", "12340"]
