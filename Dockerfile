# Build stage
FROM rust:1.90-slim AS builder

# Install build dependencies
RUN apt-get update && \
    apt-get install -y \
    libsqlite3-dev \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy all source files
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY static ./static

# Build the application
RUN cargo build --release && \
    strip target/release/yral-billing

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y \
    libsqlite3-0 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create app user
RUN groupadd -r app && useradd -r -g app app

# Create necessary directories
RUN mkdir -p /app /data && \
    chown -R app:app /app /data

WORKDIR /app

# Copy binary from builder stage
COPY --from=builder /app/target/release/yral-billing .

# Copy migrations for runtime execution
COPY --from=builder /app/migrations ./migrations

# Copy entrypoint script
COPY entrypoint.sh /app/entrypoint.sh

# Change ownership and make entrypoint executable
RUN chown -R app:app /app && \
    chmod +x /app/entrypoint.sh

USER app

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

EXPOSE 3000

# Set default environment variables (can be overridden)
ENV PORT=3000
ENV DATABASE_URL=/data/billing.db

ENTRYPOINT ["/app/entrypoint.sh"]
CMD ["./yral-billing"]