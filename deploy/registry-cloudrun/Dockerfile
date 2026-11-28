# Multi-stage Cloud Run image for s3a-registry-server
FROM rust:1.83-slim-bookworm AS rust-builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p s3a-cli

FROM python:3.12-slim-bookworm AS runtime
RUN useradd --create-home --shell /bin/bash s3a \
    && mkdir -p /app/scripts /mnt/gcs /data \
    && chown -R s3a:s3a /app /mnt/gcs /data

COPY --from=rust-builder /src/target/release/s3a-cli /usr/local/bin/s3a-cli
COPY scripts/paper_registry.py scripts/s3a_lrs_sync.py scripts/s3a_registry_server.py /app/scripts/

ENV PORT=8080 \
    PYTHONUNBUFFERED=1 \
    S3A_REGISTRY_ARCHIVE_PATH=/mnt/gcs/verified_registry.s3a \
    S3A_LRS_URL=https://34.74.163.88.sslip.io

USER s3a
WORKDIR /app
EXPOSE 8080

CMD ["python", "scripts/s3a_registry_server.py", "--host", "0.0.0.0"]
