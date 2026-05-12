# zdbsp-rust

A Rust port of [ZDBSP](https://github.com/ZDoom/zdbsp), the stand-alone version of
ZDoom's internal node builder. Reads a Doom WAD, builds BSP nodes (and optionally GL
nodes), a blockmap, and a reject table, then writes a new WAD.

The original C++ source being ported lives at `/Users/jsh/dev/repos/zdbsp/`. The goal
of this port is **byte-identical output** against the C++ baseline on the same host,
so a regression suite that diffs WAD outputs can act as the correctness oracle.

## Status

Feature-complete across the C++'s common workflows. Byte-identical against the C++
baseline on every IWAD in `~/media/doom_wads/` for these flag combinations:

| flag | identical | notes |
|------|-----------|-------|
| (default) | 7 / 7 | classic NODES/SEGS/SSECTORS/BLOCKMAP/REJECT |
| `-z` | 7 / 7 | ZNOD compressed |
| `-Z` | 7 / 7 | compress regular only |
| `-X` | 7 / 7 | XNOD extended uncompressed |
| `-P` | 7 / 7 | no polyobjects |
| `-g` | 7 / 7 | GL nodes alongside regular |
| `-G` | 7 / 7 | GL-matching (single build, derive regular) |
| `-x` | 7 / 7 | GL-only output |
| `-5` | 7 / 7 | v5 flag without GL build (no-op) |
| `-g -5`, `-G -5` | 0 / 7 | C++ writes uninitialized v5 GL_SEGS padding bytes (UB) |

**63 of 77 IWAD × flag combinations byte-identical.** Strict regression
(`ZDBSP_REGRESSION=1 cargo test`) enforces 9 flag combos × 7 IWADs.

The only remaining gap is the v5 GL_SEGS path: `MapSegGLEx`'s natural alignment leaves
a 2-byte hole between `side` and `partner`, and the C++ leaves those bytes whatever
the heap allocator returned — UB that's coincidentally deterministic on this Mac but
not byte-reproducible without exactly mimicking Apple's allocator. The Rust port
emits zero bytes there.

## Build

Standard cargo workspace, no special flags:

```sh
cargo build --release
```

The binary lands at `target/release/zdbsp`.

`flate2` is linked with the system `libz` (via `default-features = false, features =
["zlib"]`) so the compressed-node output matches the C++ baseline byte-for-byte.

## Usage

```
zdbsp [options] <input.wad>
```

Run `zdbsp --help` for the full flag list. Common invocations:

```sh
zdbsp input.wad -o output.wad           # rebuild nodes for every map
zdbsp -m MAP01 input.wad -o input.wad   # rebuild a single map in place
zdbsp -g input.wad -o output.wad        # also build GL nodes
zdbsp -z input.wad -o output.wad        # write compressed ZNODES
```

## Workspace layout

```
zdbsp-rust/
├── zdbsp-lib/             # library crate — all the algorithm code
│   ├── src/
│   │   ├── fixed.rs            # Fixed-point type + PointToAngle
│   │   ├── wad.rs              # WAD reader/writer (lump directory I/O)
│   │   ├── level.rs            # Level container + Int* data types + pruning
│   │   ├── processor.rs        # Per-map loader (Doom/Hexen binary; UDMF integration)
│   │   ├── udmf.rs             # UDMF tokenizer, parser, writer
│   │   ├── blockmap.rs         # BLOCKMAP builder (packed, hash-deduped)
│   │   ├── workdata.rs         # Internal node-builder types (Vertex, Node, Subsector)
│   │   ├── nodebuild/
│   │   │   ├── mod.rs          # NodeBuilder struct, BuildOptions, point_on_side
│   │   │   ├── types.rs        # PrivSeg, PrivVert, USegPtr, SplitSharer, PolyStart
│   │   │   ├── classify.rs     # ClassifyLine2 — partition-side classifier (scalar)
│   │   │   ├── events.rs       # FEventTree (BST keyed on partition distance)
│   │   │   ├── util.rs         # Vertex spatial hash, MakeSegsFromSides, GroupSegPlanes,
│   │   │   │                   # InterceptVector, SplitSeg, FindPolyContainers
│   │   │   ├── build.rs        # BuildTree, CreateNode, CheckSubsector, SelectSplitter,
│   │   │   │                   # Heuristic, SplitSegs, SortSegs (libc qsort via FFI)
│   │   │   ├── gl.rs           # AddIntersection, FixSplitSharers, AddMinisegs,
│   │   │   │                   # AddMiniseg, CheckLoopStart/End
│   │   │   ├── extract.rs      # GetNodes — strip minisegs, recompute short bboxes
│   │   │   └── extract_gl.rs   # GetGLNodes, CloseSubsector, OutputDegenerateSubsector
│   │   ├── writer.rs           # Top-level write_map / write_udmf_map dispatch
│   │   ├── writer_gl.rs        # GL_VERT / GL_SEGS / GL_SSECT / GL_NODES (v2 + v5)
│   │   └── writer_compressed.rs# ZNOD / ZGLN / XNOD / XGLN (flate2 + system libz)
│   └── tests/                  # integration tests against IWADs
└── zdbsp-cli/             # binary crate — `zdbsp` executable
    ├── src/main.rs             # arg parsing, lump-walk loop, per-map dispatch
    └── tests/regression.rs     # strict byte-identical regression vs C++ baseline
```

## Regression harness

The integration test at `zdbsp-cli/tests/regression.rs` invokes the C++ baseline at
`/Users/jsh/dev/repos/zdbsp/build/zdbsp`, then runs the Rust port over the same input,
and diffs the resulting WADs byte-for-byte across a matrix of flag combinations.

Two environment variables control it:
- `ZDBSP_BASELINE` — path to the C++ baseline binary (default:
  `/Users/jsh/dev/repos/zdbsp/build/zdbsp`).
- `ZDBSP_CORPUS` — directory of input WADs (default: `/Users/jsh/media/doom_wads`).

By default the harness runs in **advisory mode** (skips with a notice). Set
`ZDBSP_REGRESSION=1` to enforce byte-identical parity:

```sh
ZDBSP_REGRESSION=1 cargo test --release --test regression
```

## Translation notes

A few places where the Rust port intentionally mirrors **bugs or implementation quirks**
in the C++ baseline so the byte stream stays identical. Each is commented at its
location in the source.

- **`(uint32_t)negative_double` saturation in `PointToAngle`** — on aarch64-apple-darwin
  clang emits `fcvtzu`, which saturates negative floats to zero. Rust's `as u32`
  happens to match. A baseline rebuilt on x86_64 would produce different angles for
  segs pointing into the lower half-plane (`cvttsd2si` wraps).
- **Strict-aliasing-style sizeof bug in `WriteNodes5`** — the C++ writes
  `count * sizeof(MapNodeEx) = count * 40` bytes from a buffer of `MapNodeExO = 32`-byte
  records, leaving `count * 8` uninitialized trailing bytes per lump. Apple Silicon
  reliably zeros that memory; we emit the zeros explicitly.
- **`FixReject` opnum uses post-prune size** — the C++ indexes the old reject with the
  *new* (post-prune) sector count, which is geometrically wrong. The Rust port preserves
  the bug verbatim.
- **`StripMinisegs` side-calculation overwrite** — the C++ correctly computes
  sidedef-compression-aware `side`, then immediately overwrites it on the next line
  with the naïve check. Mirrored.
- **`OutputDegenerateSubsector` `storedseg` write target** — the C++ writes
  `seg->storedseg = PushGLSeg(...)` where `seg` is the iterator var, not `bestseg`.
  Same pattern in `CloseSubsector`'s angle-sort. Both mirrored.
- **Vertex-split truncation order in `split_segs`** — the C++ truncates `frac * delta`
  to int *first*, then adds to `v1.x`. Doing the addition in `f64` and truncating once
  rounds differently when delta is negative and `v1.x` is positive. Mirrored.
- **`SortSegs` uses libc `qsort` (unstable)** — Rust's `sort_unstable_by` doesn't
  match BSD `qsort`'s permutation of equal-keyed elements. The port routes through
  libc `qsort` via FFI so seg-ordering ties resolve identically.
- **`WriteGLVertices` truncates `fixed_t` via `LittleShort`** — the C++ writer's
  `LittleShort(vertdata[i].x)` narrows a 32-bit fixed-point coord to 16 bits, then
  zero-extends back into the 32-bit slot, losing the fractional 16 bits. We mirror
  the truncation for byte-identical GL_VERT output.
- **`OutputDegenerateSubsector` `bestinit` index** — the C++ uses
  `bestinit[bForward]` where `bForward` is `bool` indexing as `false→0→-DBL_MAX`,
  `true→1→+DBL_MAX`. An initial port had this inverted (`-MAX` for forward,
  `+MAX` for backward), which silently no-op'd the entire degenerate-subsector
  forward path and produced 7 missing GL_SEGS records per E1M1. Fixed.
- **`PushGLSeg` sidedef-compression vertex lookup** — the C++ does
  `Level.Vertices[seg->v1]` for the distance computation, after `Level.Vertices`
  was replaced with the builder's expanded array (post-`GetVertices` swap). In
  Rust, the equivalent is `self.vertices` (the builder), not `self.level.vertices`
  (still the original). An initial port mixed the two arrays, producing wrong
  `side` values on sidedef-compressed lines.

## License

GPL-2.0-or-later, matching the upstream ZDBSP.
