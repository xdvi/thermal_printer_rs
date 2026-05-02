# Benchmarks: Performance Evolution

This document tracks the cumulative improvements across optimization phases.

## Summary Matrix

| Metric | Phase 0 (Baseline) | Phase 3 (Current) | Total Improvement |
|---|---|---|---|
| **Build Text (2k)** | 5.33 µs | **4.21 µs** | **21.0% Faster** |
| **Build Receipt (8k)** | 133.85 µs | **97.03 µs** | **27.5% Faster** |
| **RAM Overhead** | 3x copies | **2x copies** | **33.3% Reduction** |
| **Stability** | No chunking | **Uniform Chunking** | **High Reliability** |

## Phase 3 Detail: Chunking Impact

| Case | Phase 2 (No Chunking) | Phase 3 (Chunking 8k) | Overhead |
|---|---|---|---|
| `send_buffer_128k` | 144.39 µs | **188.30 µs** | +30.4% |

*The chunking overhead is the cost of splitting large payloads into manageable blocks (16 chunks for 128 KiB). This is a critical stability feature for USB/BLE hardware that prevents buffer overflows in the printer.*

## Architecture Wins
- [x] **Zero-Blocking API**: No more `block_in_place`.
- [x] **Ownership-based IO**: Background task owns the transport, preventing race conditions.
- [x] **Backpressure**: Natural MPSC channel saturation prevents memory spikes.
- [x] **Cancellation**: Support for `ClearQueue` and graceful `Disconnect`.

---
*Next: Phase 4 (Platforms) will focus on Android/iOS native refinements.*
