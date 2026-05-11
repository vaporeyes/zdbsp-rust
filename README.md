# zdbsp-rust

Rust port of [ZDBSP](https://github.com/ZDoom/zdbsp), ZDoom's stand-alone node builder. The C++ source being ported lives at `/Users/jsh/dev/repos/zdbsp/`.

## Status

Phase 0: scaffolding. No algorithmic code yet.

## Layout

- `zdbsp-lib/` — library crate; will host WAD I/O, the processor, blockmap and node builder.
- `zdbsp-cli/` — binary crate producing `zdbsp`; CLI surface mirroring the C++ flags in `zdbsp.html`.
- `tests/` — integration regression harness diffing Rust output against the C++ baseline (see below).

## Porting plan

1. **Phase 0** — Workspace, C++ baseline build, diff harness.
2. **Phase 1** — WAD I/O (`wad.cpp` → `zdbsp-lib::wad`).
3. **Phase 2** — Map loading (Doom + Hexen binary formats; fixed-point `Fixed(i32)`).
4. **Phase 3** — Blockmap builder.
5. **Phase 4** — Node builder core, scalar `ClassifyLine` only.
6. **Phase 5** — GL nodes + compressed/extended (ZNODES, zlib) output.
7. **Phase 6** — Reject builder, UDMF parser, full CLI parity.
8. **Phase 7** — SIMD: SSE1/SSE2 `ClassifyLine` variants with cpuid runtime dispatch, mirroring the C++ design.

Correctness target: **byte-identical output** against the C++ baseline.

## Regression harness

`cargo test` runs unit tests plus an integration suite that:

1. Locates the C++ baseline binary (env `ZDBSP_BASELINE`, defaults to `/Users/jsh/dev/repos/zdbsp/build/zdbsp`).
2. Locates a corpus of input WADs (env `ZDBSP_CORPUS`, defaults to `/Users/jsh/media/doom_wads/`).
3. For each WAD and each flag combination it cares about, runs both binaries and diffs the outputs byte-for-byte.

If either the baseline or the corpus is missing, the suite emits a skip notice rather than failing — that way `cargo test` is meaningful in CI without bundling proprietary WADs.
