# Jackstay is one shared library behind a versioned C ABI

Jackstay is extracted as a single shared transport **library**, not a protocol
spec that each consumer reimplements. The seqlock ring, cacheline layout,
handle-passing, and sync-ordering are the code that "fails rarely and
catastrophically"; it must exist once, be ThreadSanitizer'd and model-checked
once, and be linked everywhere. Rust integrations can use the Rust API; C and
Zig consumers use the C ABI over that same implementation. The reference viewer
proves the C boundary. Direct integration by other consumers follows their own
schedules.

The implementation stays **Rust for now** (it keeps the merged control-page
seqlock work and Rust's safety on the non-hot-path management code). The C ABI —
not the implementation language — is the contract. A **C rewrite is a longer-term
option**, driven by adoption rather than by current consumers: the eventual
"drop the `.c`/`.h` into my build, zero toolchain commitment" audience. Zig was
considered and rejected — it solves Zig-consumer ergonomics, which the existing C
linkage already handles.

## 2026-09-05 sequencing amendment

[ADR-0010](0010-jackstay-extraction-and-desktop-workflow-milestones.md) separates
extraction from API stability. Extract and consume the library at 0.x, preserving
the macOS and Linux native paths. Keep the C ABI versioned; a 1.0 stability
promise and Windows native capture do not gate extraction. The shared-library
and single-implementation decisions above remain in force.

## Consequences

- **Do not fit the protocol to today's producer/consumer list** (porthole/SCK,
  katzensteg/SDL, the SDL viewer). That list is what we happen to be looking at
  now, not a model of future use. The descriptor stays general and versioned —
  full format/modifier/colorspace from day one — and the per-platform link
  surface a producer/consumer must implement stays deliberately small.
- **Extract as a usable 0.x dependency**, with standalone synthetic producer/viewer
  coverage and real macOS/Linux native integration checks. Preserve source history
  and the one-way dependency from porthole to Jackstay. The macOS native producer
  and viewer already exist; extraction must not become a new API-freeze gate.
- Interposers (producer→consumer adapters), network streamers/receivers, and a
  cross-terminal handle-passing protocol extension live in the jackstay repo as
  **longer-term** scope, not on the path to "finish A".
