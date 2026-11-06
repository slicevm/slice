# Slice (Nexus-JS) Flagship Case Studies & Interactive Workbench Makefile
# Supports both Windows (cmd.exe / PowerShell / Git Bash) and Linux / macOS.

ifeq ($(OS),Windows_NT)
    EXE := .exe
    SEP := \\
    RM_RF := cmd /c rmdir /s /q
else
    SHELL := /bin/sh
    EXE :=
    SEP := /
    RM_RF := rm -rf
endif

ROOT_DIR := ../..
CARGO ?= cargo
NPM ?= npm
DOCKER ?= docker
PORT ?= 3030
SCALE ?= 1
PROFILE ?= release

ifeq ($(PROFILE),debug)
    SLICE_BIN ?= $(ROOT_DIR)/target/debug/slice$(EXE)
    CARGO_BUILD_FLAGS := --bin slice
else
    SLICE_BIN ?= $(ROOT_DIR)/target/release/slice$(EXE)
    CARGO_BUILD_FLAGS := --release --bin slice
endif

CS01_DIR := 01-tot-autonomous-sre-agent
CS02_DIR := 02-mcp-code-mode-orchestrator
CS03_DIR := 03-multi-tenant-saas-plugin-mesh
CS04_DIR := 04-wasm-zero-trust-cdr-pipeline
CS05_DIR := 05-time-travel-chaos-fuzzer-payments
WB_DIR   := workbench

ALL_DIRS := $(CS01_DIR) $(CS02_DIR) $(CS03_DIR) $(CS04_DIR) $(CS05_DIR) $(WB_DIR)
STAMP_DEPS := $(CS01_DIR)/node_modules $(CS02_DIR)/node_modules $(CS03_DIR)/node_modules $(CS04_DIR)/node_modules $(CS05_DIR)/node_modules $(WB_DIR)/node_modules

VISUAL_FLAGS := --allow-env=SLICE_VISUAL=1 --allow-env=SLICE_LOAD_SCALE=$(SCALE) --fuel=500000000 --metrics
JSON_FLAGS   := --allow-env=SLICE_VISUAL=0 --metrics

.PHONY: help setup build install all visual cockpit stress json \
        cs01 cs02 cs03 cs04 cs05 \
        cs01-json cs02-json cs03-json cs04-json cs05-json \
        test test-cs01 test-cs02 test-cs03 test-cs04 test-cs05 \
        serve workbench smoke bench-docker clean

help:
	@echo ================================================================================
	@echo   Slice (Nexus-JS) Flagship Case Studies Makefile (Windows and Linux/macOS)
	@echo ================================================================================
	@echo   Setup and Build:
	@echo     make setup         Build release Slice binary and install all npm packages
	@echo     make build         Build Slice CLI binary (cargo build --release --bin slice)
	@echo     make install       Install npm dependencies in all 5 case studies + workbench
	@echo   ------------------------------------------------------------------------------
	@echo   Visual Execution and Heavy Stress Testing (Plain-English Explanations + Load):
	@echo     make cockpit       Run the 5-in-1 Terminal Visual Cockpit (1x default, or SCALE=N)
	@echo     make stress        Run the 5-in-1 Terminal Visual Cockpit at 10x Heavy Stress Load
	@echo     make visual        Run all 5 Case Studies individually in Visual ASCII mode
	@echo     make cs01          Run CS-01: Tree-of-Thought Autonomous SRE Agent (Visual)
	@echo     make cs02          Run CS-02: MCP Code Mode Orchestrator and DLP Firewall (Visual)
	@echo     make cs03          Run CS-03: Multi-Tenant SaaS Webhook and Plugin Mesh (Visual)
	@echo     make cs04          Run CS-04: Pure-Rust Wasm CDR and Zero-Trust PII Vault (Visual)
	@echo     make cs05          Run CS-05: Time-Travel Chaos Fuzzer for Payment Sagas (Visual)
	@echo     (Tip: Pass SCALE=5 or SCALE=10 to any target above to multiply the load!)
	@echo   ------------------------------------------------------------------------------
	@echo   Interactive Browser Workbench:
	@echo     make serve         Start live Workbench UI at http://127.0.0.1:$(PORT)
	@echo     make smoke         Run headless HTTP smoke-test against the Workbench UI
	@echo   ------------------------------------------------------------------------------
	@echo   Deterministic JSON Output and Time-Travel Tests:
	@echo     make json          Run all 5 Case Studies in canonical JSON benchmark mode
	@echo     make cs01-json     Run CS-01..CS-05 in canonical JSON mode (cs01-json..cs05-json)
	@echo     make test          Run slice test Time-Travel suites across all 5 Case Studies
	@echo   ------------------------------------------------------------------------------
	@echo   Cross-Runtime Docker Benchmarks (Slice vs Bun vs Deno vs Node.js):
	@echo     make bench-docker  Build and run 4-way Docker benchmark (SLICE_BENCH_TAGS=case_studies)
	@echo ================================================================================

setup: build install

build:
	$(CARGO) build --manifest-path $(ROOT_DIR)/Cargo.toml $(CARGO_BUILD_FLAGS)

$(SLICE_BIN):
	$(CARGO) build --manifest-path $(ROOT_DIR)/Cargo.toml $(CARGO_BUILD_FLAGS)

$(CS01_DIR)/node_modules:
	cd $(CS01_DIR) && $(NPM) install --no-audit --no-fund

$(CS02_DIR)/node_modules:
	cd $(CS02_DIR) && $(NPM) install --no-audit --no-fund

$(CS03_DIR)/node_modules:
	cd $(CS03_DIR) && $(NPM) install --no-audit --no-fund

$(CS04_DIR)/node_modules:
	cd $(CS04_DIR) && $(NPM) install --no-audit --no-fund

$(CS05_DIR)/node_modules:
	cd $(CS05_DIR) && $(NPM) install --no-audit --no-fund

$(WB_DIR)/node_modules:
	cd $(WB_DIR) && $(NPM) install --no-audit --no-fund

install:
	cd $(CS01_DIR) && $(NPM) install --no-audit --no-fund
	cd $(CS02_DIR) && $(NPM) install --no-audit --no-fund
	cd $(CS03_DIR) && $(NPM) install --no-audit --no-fund
	cd $(CS04_DIR) && $(NPM) install --no-audit --no-fund
	cd $(CS05_DIR) && $(NPM) install --no-audit --no-fund
	cd $(WB_DIR) && $(NPM) install --no-audit --no-fund

# ------------------------------------------------------------------------------
# Visual Execution Targets
# ------------------------------------------------------------------------------

cockpit: $(SLICE_BIN) $(STAMP_DEPS)
	$(SLICE_BIN) run $(WB_DIR)/cli_cockpit.ts --allow-read=/tmp --allow-read=. --allow-write=/tmp --allow-env=SLICE_LOAD_SCALE=$(SCALE) --fuel=500000000 --metrics

stress: $(SLICE_BIN) $(STAMP_DEPS)
	$(SLICE_BIN) run $(WB_DIR)/cli_cockpit.ts --allow-read=/tmp --allow-read=. --allow-write=/tmp --allow-env=SLICE_LOAD_SCALE=10 --fuel=500000000 --metrics

visual: cs01 cs02 cs03 cs04 cs05

cs01: $(SLICE_BIN) $(CS01_DIR)/node_modules
	$(SLICE_BIN) run $(CS01_DIR)/src/index.ts $(VISUAL_FLAGS)

cs02: $(SLICE_BIN) $(CS02_DIR)/node_modules
	$(SLICE_BIN) run $(CS02_DIR)/src/index.ts $(VISUAL_FLAGS)

cs03: $(SLICE_BIN) $(CS03_DIR)/node_modules
	$(SLICE_BIN) run $(CS03_DIR)/src/index.ts $(VISUAL_FLAGS)

cs04: $(SLICE_BIN) $(CS04_DIR)/node_modules
	$(SLICE_BIN) run $(CS04_DIR)/src/index.ts $(VISUAL_FLAGS)

cs05: $(SLICE_BIN) $(CS05_DIR)/node_modules
	$(SLICE_BIN) run $(CS05_DIR)/src/index.ts $(VISUAL_FLAGS)

# ------------------------------------------------------------------------------
# Canonical JSON Benchmark Output Targets
# ------------------------------------------------------------------------------

all: json

json: cs01-json cs02-json cs03-json cs04-json cs05-json

cs01-json: $(SLICE_BIN) $(CS01_DIR)/node_modules
	$(SLICE_BIN) run $(CS01_DIR)/src/index.ts $(JSON_FLAGS)

cs02-json: $(SLICE_BIN) $(CS02_DIR)/node_modules
	$(SLICE_BIN) run $(CS02_DIR)/src/index.ts $(JSON_FLAGS)

cs03-json: $(SLICE_BIN) $(CS03_DIR)/node_modules
	$(SLICE_BIN) run $(CS03_DIR)/src/index.ts $(JSON_FLAGS)

cs04-json: $(SLICE_BIN) $(CS04_DIR)/node_modules
	$(SLICE_BIN) run $(CS04_DIR)/src/index.ts $(JSON_FLAGS)

cs05-json: $(SLICE_BIN) $(CS05_DIR)/node_modules
	$(SLICE_BIN) run $(CS05_DIR)/src/index.ts $(JSON_FLAGS)

# ------------------------------------------------------------------------------
# Time-Travel Test Suites (slice test)
# ------------------------------------------------------------------------------

test: test-cs01 test-cs02 test-cs03 test-cs04 test-cs05 smoke

test-cs01: $(SLICE_BIN) $(CS01_DIR)/node_modules
	$(SLICE_BIN) test $(CS01_DIR)

test-cs02: $(SLICE_BIN) $(CS02_DIR)/node_modules
	$(SLICE_BIN) test $(CS02_DIR)

test-cs03: $(SLICE_BIN) $(CS03_DIR)/node_modules
	$(SLICE_BIN) test $(CS03_DIR)

test-cs04: $(SLICE_BIN) $(CS04_DIR)/node_modules
	$(SLICE_BIN) test $(CS04_DIR)

test-cs05: $(SLICE_BIN) $(CS05_DIR)/node_modules
	$(SLICE_BIN) test $(CS05_DIR)

# ------------------------------------------------------------------------------
# Interactive HTTP Browser Workbench
# ------------------------------------------------------------------------------

workbench: serve

serve: $(SLICE_BIN) $(STAMP_DEPS)
	$(SLICE_BIN) serve $(WB_DIR)/src/index.ts --port=$(PORT) --allow-read=/tmp --allow-read=. --allow-write=/tmp --fuel=500000000

smoke: $(SLICE_BIN) $(STAMP_DEPS)
	$(SLICE_BIN) serve $(WB_DIR)/src/index.ts --port=3199 --allow-read=/tmp --allow-read=. --allow-write=/tmp --fuel=500000000 --smoke-test

# ------------------------------------------------------------------------------
# Cross-Runtime Docker Benchmark Suite (Slice vs Bun vs Deno vs Node.js)
# ------------------------------------------------------------------------------

bench-docker:
	$(DOCKER) build -t slice-bench -f $(ROOT_DIR)/tests/Dockerfile $(ROOT_DIR)
	$(DOCKER) run --rm -e SLICE_BENCH_TAGS=case_studies slice-bench

clean:
	-$(RM_RF) "$(CS01_DIR)$(SEP)node_modules"
	-$(RM_RF) "$(CS02_DIR)$(SEP)node_modules"
	-$(RM_RF) "$(CS03_DIR)$(SEP)node_modules"
	-$(RM_RF) "$(CS04_DIR)$(SEP)node_modules"
	-$(RM_RF) "$(CS05_DIR)$(SEP)node_modules"
	-$(RM_RF) "$(WB_DIR)$(SEP)node_modules"
