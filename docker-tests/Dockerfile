FROM rust:latest AS builder

WORKDIR /workspace
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    ca-certificates \
    unzip \
    xz-utils \
    time \
    procps \
    gawk \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js v22, Bun, and Deno for side-by-side real-stack production comparison
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://bun.sh/install | bash
ENV PATH="/root/.bun/bin:${PATH}"

RUN curl -fsSL https://deno.land/install.sh | sh
ENV PATH="/root/.deno/bin:${PATH}"

WORKDIR /app
COPY --from=builder /workspace/target/release/slice /app/target/release/slice
COPY benchmarks /app/benchmarks
COPY scripts /app/scripts
COPY docker-tests /app/docker-tests
RUN sed -i 's/\r$//' /app/scripts/benchmark_linux.sh /app/docker-tests/run_real_stack_benchmarks.sh \
    && chmod +x /app/scripts/benchmark_linux.sh /app/docker-tests/run_real_stack_benchmarks.sh

CMD ["/app/docker-tests/run_real_stack_benchmarks.sh"]

