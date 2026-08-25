# ---- Build stage ----
FROM rust:1-slim-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml ./
COPY src ./src
COPY static ./static

RUN cargo build --release

# ---- Runtime stage ----
FROM debian:bookworm-slim

ARG KUBECTL_VERSION=v1.30.0
ARG TARGETARCH

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && ARCH="${TARGETARCH:-$(dpkg --print-architecture)}" \
    && case "$ARCH" in \
        amd64|arm64) ;; \
        *) echo "unsupported architecture: $ARCH" >&2; exit 1 ;; \
    esac \
    && curl -fsSL -o /tmp/kubectl \
       "https://dl.k8s.io/release/${KUBECTL_VERSION}/bin/linux/${ARCH}/kubectl" \
    && curl -fsSL -o /tmp/kubectl.sha256 \
       "https://dl.k8s.io/release/${KUBECTL_VERSION}/bin/linux/${ARCH}/kubectl.sha256" \
    && echo "$(cat /tmp/kubectl.sha256)  /tmp/kubectl" | sha256sum -c - \
    && mv /tmp/kubectl /usr/local/bin/kubectl \
    && chmod +x /usr/local/bin/kubectl \
    && rm -f /tmp/kubectl.sha256 \
    && apt-get purge -y curl \
    && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd -r appuser \
    && useradd -r -g appuser -u 1000 --create-home appuser \
    && mkdir -p /app/data /home/appuser/.kube \
    && chown -R appuser:appuser /app/data /home/appuser/.kube

COPY --from=builder /app/target/release/openab-dashboard /app/openab-dashboard
COPY docker-entrypoint.sh /app/docker-entrypoint.sh
RUN chmod +x /app/docker-entrypoint.sh

USER appuser

# The runtime process starts in /app/data; database.path "./usage.db" in
# config.example.yaml resolves to /app/data/usage.db. docker-compose.yml mounts
# the named volume "openab-data" at /app/data.
WORKDIR /app/data
VOLUME ["/app/data"]

EXPOSE 8080

ENTRYPOINT ["/app/docker-entrypoint.sh"]
CMD ["--config", "/app/config.yaml"]
