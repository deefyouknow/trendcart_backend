# Use the official Rust image as a builder
FROM rust:1.80-alpine AS builder

# Set the working directory inside the container
WORKDIR /usr/src/app

# Install required build dependencies for alpine
RUN apk add --no-cache musl-dev pkgconfig openssl-dev

# Copy the Cargo.toml and Cargo.lock files
COPY Cargo.toml Cargo.lock ./

# Create a dummy src directory to cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies only (this step will be cached)
RUN cargo build --release
RUN rm -rf src

# Copy the actual source code
COPY src ./src
COPY .sqlx ./.sqlx

# Build the application
# Use the offline mode for sqlx to build without needing a live database
ENV SQLX_OFFLINE=true
RUN cargo build --release

# Use a minimal alpine image for the runtime
FROM alpine:3.19

# Set the working directory inside the container
WORKDIR /app

# Install necessary runtime dependencies (e.g., ca-certificates for HTTPS)
RUN apk add --no-cache ca-certificates tzdata

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/app/target/release/backend /usr/local/bin/trendcart_api

# Set permissions and ownership (optional but recommended for security)
RUN addgroup -S appgroup && adduser -S appuser -G appgroup
RUN chown appuser:appgroup /usr/local/bin/trendcart_api
USER appuser

# Expose the application port
EXPOSE 59123

# Set environment variables (these will typically be overridden at runtime)
ENV RUST_LOG=info

# Run the backend binary
CMD ["trendcart_api"]
