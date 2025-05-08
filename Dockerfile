FROM rust:slim-bullseye as builder

WORKDIR /usr/src/app

# Create a dummy project with the Cargo.toml and Cargo.lock
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build dependencies
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src/

# Copy the actual source code
COPY src/ src/

# Build the application with full source code
RUN touch src/main.rs && \
    cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install SSL certificates and other runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl-dev \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from the builder stage
COPY --from=builder /usr/src/app/target/release/litegate /app/litegate

# Copy the .env file template
COPY .env.sample /app/.env

# Create directory for the database
RUN mkdir -p /app/data

# Set default environment variables
ENV DB_FILE=/app/data/payments.db
ENV PORT=8000

# Expose the port
EXPOSE 8000

# Run the application
CMD ["./litegate"] 