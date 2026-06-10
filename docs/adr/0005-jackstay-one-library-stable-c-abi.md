# Jackstay is one shared library behind a stable C ABI

Jackstay is extracted as a single shared transport **library**, not a protocol
spec that each consumer reimplements. The seqlock ring, cacheline layout,
handle-passing, and sync-ordering are the code that "fails rarely and
catastrophically"; it must exist once, be ThreadSanitizer'd and model-checked
once, and be linked everywhere. Porthole (Rust), katzensteg (Zig), the
libghostty-vt fork (Zig), and the in-repo `capture-viewer-sdl` (C) all consume
the *same* implementation through its C ABI — they do not carry divergent copies.

The implementation stays **Rust for now** (it keeps the merged control-page
seqlock work and Rust's safety on the non-hot-path management code). The C ABI —
not the implementation language — is the contract. A **C rewrite is a longer-term
option**, driven by adoption rather than by current consumers: the eventual
"drop the `.c`/`.h` into my build, zero toolchain commitment" audience. Zig was
considered and rejected — it solves Zig-consumer ergonomics, which the existing C
linkage already handles.

## Consequences

- **Do not fit the protocol to today's producer/consumer list** (porthole/SCK,
  katzensteg/SDL, the SDL viewer). That list is what we happen to be looking at
  now, not a model of future use. The descriptor stays general and versioned —
  full format/modifier/colorspace from day one — and the per-platform link
  surface a producer/consumer must implement stays deliberately small.
- **Extract the repo only after the macOS native path is proven** (see ADR-0004).
  The crate is already a clean, one-way-dependency boundary (`porthole` →
  `capture-transfer`, never the reverse), so extraction is a history-preserving
  move, not a refactor — but doing it before native exists risks freezing a
  CPU-shaped API that the handle/fence/pool model then has to fight.
- Interposers (producer→consumer adapters), network streamers/receivers, and a
  cross-terminal handle-passing protocol extension live in the jackstay repo as
  **longer-term** scope, not on the path to "finish A".
