# LSP Proxy Benchmark: Node.js vs Rust

## Overview

This report compares the performance of the existing Node.js LSP proxy (`proxy.mjs`) against the new native Rust replacement (`java-lsp-proxy`). Both proxies sit between Zed and JDTLS, forwarding LSP messages bidirectionally, sorting completion responses by parameter count, and exposing an HTTP server for extension-originated requests.

## Methodology

- Both proxies were instrumented with high-resolution timing (nanosecond on JS via `hrtime.bigint()`, microsecond on Rust via `std::time::Instant`)
- Benchmarking was gated behind `LSP_PROXY_BENCH=1` — zero overhead when disabled
- Each message records: direction, LSP method, payload size, and proxy processing overhead in microseconds
- Overhead measures only the proxy's own processing time (parse → transform → forward), excluding JDTLS response latency
- Tests were run on the same machine (macOS, Apple Silicon) with the same Zed configuration and JDTLS version, performing typical editing workflows: navigation, completions, saves, diagnostics

## Test Environment

| | Details |
|---|---|
| Machine | macOS, Apple Silicon (aarch64) |
| JDTLS | 1.57.0-202602261110 |
| Node.js | v24.11.0 (Zed-bundled) |
| Rust proxy | Release build, 771 KB binary |
| Zed | Dev extension |

## Results

### Node.js Proxy (3,700 messages)

| Direction | Count | Min (µs) | Median (µs) | P95 (µs) | P99 (µs) | Max (µs) | Avg (µs) |
|---|---:|---:|---:|---:|---:|---:|---:|
| client → server | 1,399 | 3 | 16 | 81 | 147 | 429 | 28 |
| server → client | 2,011 | 4 | 24 | 74 | 121 | 4,501 | 32 |
| server → client (completion) | 290 | 13 | 50 | 179 | 272 | 458 | 71 |
| **Total** | **3,700** | | | | | | **33** |

Total overhead: **124,796 µs** (~125 ms)

### Rust Proxy (5,277 messages)

| Direction | Count | Min (µs) | Median (µs) | P95 (µs) | P99 (µs) | Max (µs) | Avg (µs) |
|---|---:|---:|---:|---:|---:|---:|---:|
| client → server | 2,093 | 0 | 7 | 32 | 58 | 269 | 10 |
| server → client | 2,666 | 1 | 8 | 32 | 63 | 1,185 | 12 |
| server → client (completion) | 523 | 4 | 17 | 116 | 143 | 253 | 29 |
| **Total** | **5,277** | | | | | | **13** |

Total overhead: **72,026 µs** (~72 ms)

### Head-to-Head Comparison (Median)

| Direction | Node.js | Rust | Speedup |
|---|---:|---:|---:|
| client → server (passthrough) | 16 µs | 7 µs | **2.3x** |
| server → client (passthrough) | 24 µs | 8 µs | **3.0x** |
| server → client (completion sort) | 50 µs | 17 µs | **2.9x** |
| **Overall average** | **33 µs** | **13 µs** | **2.5x** |

### Tail Latency Comparison (P99)

| Direction | Node.js | Rust | Improvement |
|---|---:|---:|---:|
| client → server | 147 µs | 58 µs | **2.5x** |
| server → client | 121 µs | 63 µs | **1.9x** |
| server → client (completion sort) | 272 µs | 143 µs | **1.9x** |

## Analysis

- The Rust proxy is **2.5x faster on average** across all message types
- The completion sorting path — which involves full JSON parse, field mutation, and re-serialization — shows a **2.9x improvement** at the median (17 µs vs 50 µs)
- Tail latency (P99) is **~2x tighter** in Rust, meaning more predictable performance
- Both proxies add negligible latency compared to JDTLS response times (typically 10-500 ms), so the user-perceived difference is minimal
- The primary benefits of the Rust proxy are architectural:
  - **No Node.js runtime dependency** — eliminates ~50 MB runtime
  - **771 KB static binary** — trivial to distribute
  - **Faster cold start** — no V8 JIT warmup
  - **Lower memory footprint** — no garbage collector overhead
  - **Cross-compiled** — single binary per platform via CI

## Appendix: Message Size Distribution (Rust run)

| Direction | Min | Median | Max | Avg |
|---|---:|---:|---:|---:|
| client → server | 62 B | 381 B | 6,151 B | 366 B |
| server → client | 49 B | 549 B | 50,675 B | 1,228 B |
| server → client (completion) | 58 B | 110 B | 25,049 B | 2,060 B |
