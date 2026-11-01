# syntax=docker/dockerfile:1

# -------- Builder stage --------
FROM rust:1-bookworm AS builder
WORKDIR /app

# Cache dependencies: build them against stub sources so this layer is
# only invalidated when Cargo.toml / Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
 && echo "fn main() {}" > src/main.rs \
 && touch src/lib.rs \
 && cargo build --release --locked \
 && rm -rf src

# Now copy the real source
COPY src ./src

# Build the actual release binary (touch so cargo rebuilds over the stubs)
RUN touch src/main.rs src/lib.rs && cargo build --release --locked

# -------- Runtime stage --------
FROM debian:bookworm-slim AS runtime

# Install certificates for HTTPS and clean up apt caches
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 10001 appuser

# Copy the compiled binary
COPY --from=builder /app/target/release/crawl /usr/local/bin/crawl

# Ensure runtime directory for optional output mounts
RUN mkdir -p /data && chown appuser:appuser /data
WORKDIR /data

# Environment variables (optional defaults)
# You can override these at `docker run` time.
# ENV CRAWL_START_URL=
# ENV CRAWL_DEPTH=2
# ENV CRAWL_SCOPE=
# ENV CRAWL_SUBDOMAINS=false
# ENV CRAWL_DELAY_MS=1000
# ENV CRAWL_USER_AGENT=rust-crawler/0.2.0
# ENV CRAWL_CONCURRENCY=10
# ENV CRAWL_OUTPUT=
# ENV CRAWL_QUIET=false
# ENV CRAWL_TIMEOUT_SECS=30
# ENV CRAWL_MAX_RETRIES=3
# ENV CRAWL_IGNORE_ROBOTS=false
# ENV CRAWL_SITEMAP=false
# ENV CRAWL_MAX_PAGES=

# Drop privileges
USER appuser

# Default entrypoint runs the crawler; provide args or env vars
ENTRYPOINT ["/usr/local/bin/crawl"]
