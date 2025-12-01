# Build stage
FROM rust:latest as builder

WORKDIR /app

# Copy manifest files
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src
COPY migrations ./migrations

# Build the application
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install required dependencies
RUN apt-get update && \
    apt-get install -y \
    libsqlite3-0 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create app user
RUN groupadd -r app && useradd -r -g app app

# Create data directory
RUN mkdir -p /data && chown app:app /data

WORKDIR /app

# Copy binary from builder stage
COPY --from=builder /app/target/release/yral-billing .

# Change ownership
RUN chown -R app:app /app

USER app

EXPOSE 3000

CMD ["./yral-billing"]