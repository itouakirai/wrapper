# The published image (ghcr.io/itouakirai/wrapper): the linux-x86_64 package
# CI already built, without compiling anything again. The build context is
# the unpacked package (`wrapper` + `rootfs/`); see the docker job in
# .github/workflows/build-lite.yml. To build from source use ../Dockerfile.
FROM debian:13-slim
WORKDIR /app
COPY rootfs /app/rootfs
COPY wrapper /app/wrapper
# Account data (logins, sessions, statistics, settings): mount it.
VOLUME /app/rootfs/data
EXPOSE 12340
ENV WRAPPER_HOST=0.0.0.0 WRAPPER_PORT=12340
CMD ["/app/wrapper", "agent"]
