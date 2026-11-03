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
    jq \
    libatomic1 \
    time \
    procps \
    gawk \
    && rm -rf /var/lib/apt/lists/*

# 1. Install the ABSOLUTE LATEST official release of Node.js directly from nodejs.org/dist/index.json
RUN NODE_LATEST=$(curl -fsSL https://nodejs.org/dist/index.json | jq -r '.[0].version') \
    && echo "Installing latest Node.js: ${NODE_LATEST}" \
    && curl -fsSL "https://nodejs.org/dist/${NODE_LATEST}/node-${NODE_LATEST}-linux-x64.tar.xz" \
       | tar -xJ --strip-components=1 -C /usr/local

# 2. Install the ABSOLUTE LATEST official release of Bun
RUN curl -fsSL https://bun.sh/install | bash
ENV PATH="/root/.bun/bin:${PATH}"

# 3. Install the ABSOLUTE LATEST official release of Deno
RUN curl -fsSL https://deno.land/install.sh | sh
ENV PATH="/root/.deno/bin:${PATH}"

WORKDIR /app
COPY --from=builder /workspace/target/release/slice /app/target/release/slice
COPY benchmarks /app/benchmarks
COPY scripts /app/scripts
COPY tests /app/tests

# Install real external npm packages in the folder-based test applications
RUN bun install --cwd /app/tests/apps/02-npm-ecosystem-suite \
    && bun install --cwd /app/tests/apps/04-edge-api-gateway \
    && bun install --cwd /app/tests/apps/05-jwt-oauth-session-guard \
    && bun install --cwd /app/tests/apps/06-telemetry-metrics-pipeline \
    && bun install --cwd /app/tests/apps/07-graphql-ast-validator \
    && bun install --cwd /app/tests/apps/08-event-driven-state-journal \
    && bun install --cwd /app/tests/apps/10-fullstack-bff-microservice \
    && bun install --cwd /app/tests/apps/11-static-asset-cdn-edge \
    && bun install --cwd /app/tests/apps/12-ai-sse-token-streamer \
    && bun install --cwd /app/tests/apps/13-webhook-orchestrator-ssrf \
    && bun install --cwd /app/tests/apps/14-federated-api-mesh-dlp \
    && bun install --cwd /app/tests/apps/15-time-travel-test-suite \
    && bun install --cwd /app/tests/apps/16-dotenv-dlp-config-vault \
    && bun install --cwd /app/tests/apps/17-zero-trust-redirect-keepalive-mesh \
    && bun install --cwd /app/tests/apps/18-async-event-loop-fanout-scheduler \
    && bun install --cwd /app/tests/apps/19-wintertc-webcrypto-streams-edge \
    && bun install --cwd /app/tests/apps/20-node-compat-and-edge-router \
    && bun install --cwd /app/tests/apps/21-websocket-realtime-and-security-guard \
    && bun install --cwd /app/tests/apps/22-worker-threads-and-raw-sockets \
    && bun install --cwd /app/tests/apps/23-wasm-crypto-and-simd-kernel \
    && bun install --cwd /app/tests/apps/24-tsx-ssr-and-wasm-edge-pipeline \
    && sed -i 's/\r$//' /app/scripts/benchmark_linux.sh /app/tests/run_real_stack_benchmarks.sh \
    && chmod +x /app/scripts/benchmark_linux.sh /app/tests/run_real_stack_benchmarks.sh

CMD ["/app/tests/run_real_stack_benchmarks.sh"]
