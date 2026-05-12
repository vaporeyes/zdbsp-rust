// ABOUTME: zdbsp CLI. Minimal arg parsing matching the subset of flags supported by the
// ABOUTME: Rust port so far: input wad, -o output, -m map, -N no-nodes, -q no-prune,
// ABOUTME: -r reject-zero, -R reject-zeroes, -b blockmap-zero.

use std::path::PathBuf;
use std::process::ExitCode;

use zdbsp_lib::nodebuild::{util::collect_poly_spots, NodeBuilder};
use zdbsp_lib::processor::Processor;
use zdbsp_lib::wad::{WadReader, WadWriter};
use zdbsp_lib::writer::{self, BlockmapMode, RejectMode, WriterOptions};

#[derive(Debug, Default)]
struct Args {
    input: Option<PathBuf>,
    output: PathBuf,
    map_filter: Option<String>,
    no_prune: bool,
    build_nodes: bool,
    build_gl_nodes: bool,
    check_polyobjs: bool,
    compress_nodes: bool,
    compress_gl_nodes: bool,
    force_compression: bool,
    write_comments: bool,
    gl_only: bool,
    conform_nodes: bool,
    v5_gl_nodes: bool,
    reject_mode: RejectMode,
    blockmap_mode: BlockmapMode,
    max_segs: Option<i32>,
    split_cost: Option<i32>,
    aa_preference: Option<i32>,
}

fn print_help() {
    print!(
        "ZDBSP-rust 0.1.0 — Rust port of ZDoom's node builder

Usage: zdbsp [options] <input.wad>

Output:
  -o, --output=FILE        Write to FILE (default: tmp.wad)
  -m, --map=MAP            Process only the named map (e.g. E1M1, MAP01)
  -V, --version            Print version and exit
  -h, --help               Print this message and exit

Node builder:
  -N, --no-nodes           Do not build nodes; copy existing NODES/SEGS/SSECTORS
  -q, --no-prune           Keep unreferenced sides/sectors
  -g, --gl                 Also build GL nodes (writes GL_VERT/GL_SEGS/GL_SSECT/GL_NODES)
  -G, --gl-matching        Build GL nodes once; derive regular nodes from the same build
  -x, --gl-only            Build only GL nodes; regular lumps are emitted empty
  -5, --gl-v5              Force v5 GL node record layout (also auto-promoted on large maps)

Compression / extended formats:
  -z, --compress           Force ZNOD/ZGL* (zlib-compressed extended) output
  -Z, --compress-normal    Compress only the regular NODES; leave GL uncompressed
  -X, --extended           Force XNOD/XGL* (uncompressed extended) output

Reject:
  -r, --empty-reject       Emit a zero-length REJECT lump
  -R, --zero-reject        Emit a REJECT filled with zero bytes
  -e, --full-reject        Rebuild REJECT (UNSUPPORTED; warns and falls back to --no-reject)
  -E, --no-reject          Copy REJECT through unchanged (default)

Blockmap:
  -b, --empty-blockmap     Emit a zero-length BLOCKMAP lump (game will rebuild)

Polyobjects:
  -P, --no-polyobjs        Do not look for polyobject containers during partition selection

UDMF (text-format maps):
  -c, --comments           Annotate UDMF blocks with `// <index>` comments

Build heuristics:
  -p, --partition=N        Split a node when its set exceeds N segs (default: 64, min: 3)
  -s, --split-cost=N       Score penalty per split when picking a partition (default: 8, min: 1)
  -d, --diagonal-cost=N    Axis-aligned splitter preference weight (default: 16, min: 1)
"
    );
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        output: PathBuf::from("tmp.wad"),
        build_nodes: true,
        check_polyobjs: true,
        ..Args::default()
    };

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut it = raw.into_iter();
    while let Some(arg) = it.next() {
        if let Some(rest) = arg.strip_prefix("-o") {
            if rest.is_empty() {
                a.output = it.next().ok_or("-o expects a filename")?.into();
            } else {
                a.output = rest.into();
            }
        } else if let Some(rest) = arg.strip_prefix("--output=") {
            a.output = rest.into();
        } else if let Some(rest) = arg.strip_prefix("-m") {
            if rest.is_empty() {
                a.map_filter = Some(it.next().ok_or("-m expects a map name")?);
            } else {
                a.map_filter = Some(rest.to_string());
            }
        } else if let Some(rest) = arg.strip_prefix("--map=") {
            a.map_filter = Some(rest.to_string());
        } else if arg == "-N" || arg == "--no-nodes" {
            a.build_nodes = false;
        } else if arg == "-g" || arg == "--gl" {
            // C++ main.cpp:377-380.
            a.build_gl_nodes = true;
            a.conform_nodes = false;
        } else if arg == "-G" || arg == "--gl-matching" {
            // C++ main.cpp:381-384: build GL once, derive regular from it.
            a.build_gl_nodes = true;
            a.conform_nodes = true;
        } else if arg == "-x" || arg == "--gl-only" {
            // C++ main.cpp:400-404: GL nodes only.
            a.gl_only = true;
            a.build_gl_nodes = true;
            a.conform_nodes = false;
        } else if arg == "-5" || arg == "--gl-v5" {
            // C++ main.cpp:405-407.
            a.v5_gl_nodes = true;
        } else if arg == "-z" || arg == "--compress" {
            // C++ main.cpp:390-394: compress both, force ZLib output.
            a.compress_nodes = true;
            a.compress_gl_nodes = true;
            a.force_compression = true;
        } else if arg == "-Z" || arg == "--compress-normal" {
            // C++ main.cpp:395-399: compress only regular NODES.
            a.compress_nodes = true;
            a.compress_gl_nodes = false;
            a.force_compression = true;
        } else if arg == "-X" || arg == "--extended" {
            // C++ main.cpp:385-389: extended uncompressed (XNOD/XGLN).
            a.compress_nodes = true;
            a.compress_gl_nodes = true;
            a.force_compression = false;
        } else if arg == "-q" || arg == "--no-prune" {
            a.no_prune = true;
        } else if arg == "-r" || arg == "--empty-reject" {
            a.reject_mode = RejectMode::Create0;
        } else if arg == "-R" || arg == "--zero-reject" {
            a.reject_mode = RejectMode::CreateZeroes;
        } else if arg == "-e" || arg == "--full-reject" {
            a.reject_mode = RejectMode::Rebuild;
        } else if arg == "-E" || arg == "--no-reject" {
            a.reject_mode = RejectMode::DontTouch;
        } else if arg == "-b" || arg == "--empty-blockmap" {
            a.blockmap_mode = BlockmapMode::Create0;
        } else if arg == "-P" || arg == "--no-polyobjs" {
            a.check_polyobjs = false;
        } else if arg == "-c" || arg == "--comments" {
            a.write_comments = true;
        } else if let Some(rest) = arg.strip_prefix("-p") {
            let v = if rest.is_empty() {
                it.next().ok_or("-p expects a number")?
            } else {
                rest.to_string()
            };
            let n: i32 = v.parse().map_err(|_| "-p: not a number")?;
            a.max_segs = Some(n.max(3));
        } else if let Some(rest) = arg.strip_prefix("--partition=") {
            let n: i32 = rest.parse().map_err(|_| "--partition: not a number")?;
            a.max_segs = Some(n.max(3));
        } else if let Some(rest) = arg.strip_prefix("-s") {
            let v = if rest.is_empty() {
                it.next().ok_or("-s expects a number")?
            } else {
                rest.to_string()
            };
            let n: i32 = v.parse().map_err(|_| "-s: not a number")?;
            a.split_cost = Some(n.max(1));
        } else if let Some(rest) = arg.strip_prefix("--split-cost=") {
            let n: i32 = rest.parse().map_err(|_| "--split-cost: not a number")?;
            a.split_cost = Some(n.max(1));
        } else if let Some(rest) = arg.strip_prefix("-d") {
            let v = if rest.is_empty() {
                it.next().ok_or("-d expects a number")?
            } else {
                rest.to_string()
            };
            let n: i32 = v.parse().map_err(|_| "-d: not a number")?;
            a.aa_preference = Some(n.max(1));
        } else if let Some(rest) = arg.strip_prefix("--diagonal-cost=") {
            let n: i32 = rest.parse().map_err(|_| "--diagonal-cost: not a number")?;
            a.aa_preference = Some(n.max(1));
        } else if arg == "--version" || arg == "-V" {
            println!("ZDBSP-rust 0.1.0");
            std::process::exit(0);
        } else if arg == "--help" || arg == "-h" {
            print_help();
            std::process::exit(0);
        } else if arg.starts_with('-') {
            return Err(format!("unsupported flag: {arg}"));
        } else {
            if a.input.is_some() {
                return Err(format!("unexpected positional argument: {arg}"));
            }
            a.input = Some(arg.into());
        }
    }
    if a.input.is_none() {
        return Err("missing input wad".into());
    }
    Ok(a)
}

fn run(args: Args) -> Result<(), String> {
    let input = args.input.as_deref().unwrap();
    if input == args.output {
        return Err("input and output must differ (in-place output not yet supported)".into());
    }

    let opts = WriterOptions {
        blockmap_mode: args.blockmap_mode,
        reject_mode: args.reject_mode,
        build_nodes: args.build_nodes,
        build_gl_nodes: args.build_gl_nodes,
        compress_nodes: args.compress_nodes,
        compress_gl_nodes: args.compress_gl_nodes,
        force_compression: args.force_compression,
        write_comments: args.write_comments,
        gl_only: args.gl_only,
        conform_nodes: args.conform_nodes,
        v5_gl_nodes: args.v5_gl_nodes,
    };

    // Build-time tunables for the NodeBuilder. None means "leave default".
    let build_opts = zdbsp_lib::nodebuild::BuildOptions {
        max_segs: args.max_segs.unwrap_or(64),
        split_cost: args.split_cost.unwrap_or(8),
        aa_preference: args.aa_preference.unwrap_or(16),
    };

    let mut reader = WadReader::open(input).map_err(|e| format!("open input: {e}"))?;
    let is_iwad = reader.is_iwad();
    let num_lumps = reader.num_lumps();
    let mut out = WadWriter::create(&args.output, is_iwad).map_err(|e| format!("create output: {e}"))?;

    // Mirror the C++ main loop (main.cpp:229-260): walk lumps, rebuild matching maps,
    // skip GL-nodes input for maps we rebuild, copy everything else through.
    let mut lump = 0i32;
    while lump < num_lumps {
        let is_map = reader.is_map(lump);
        let matches_filter = match &args.map_filter {
            Some(m) => {
                let name = reader.lump_name(lump);
                m.eq_ignore_ascii_case(&name)
            }
            None => true,
        };
        if is_map && matches_filter {
            let map_name = reader.lump_name(lump).into_owned();
            eprintln!("----{map_name}----");
            if reader.is_udmf(lump) {
                let processor = Processor::load(&mut reader, lump, args.no_prune)
                    .map_err(|e| format!("load {map_name}: {e}"))?;
                writer::write_udmf_map(&mut out, &mut reader, lump, &processor.level, opts)
                    .map_err(|e| format!("write {map_name}: {e}"))?;
                lump = reader.lump_after_map(lump);
                continue;
            }
            let mut processor = Processor::load(&mut reader, lump, args.no_prune)
                .map_err(|e| format!("load {map_name}: {e}"))?;

            // Build dispatch matrix mirroring processor.cpp:601-643:
            //
            // | flags          | builds | extracts |
            // |----------------|--------|----------|
            // | (default)      | reg    | reg      |
            // | -g             | gl, reg| reg, gl  |  (two builds)
            // | -G  (conform)  | gl     | reg, gl  |  (one build, derive regular)
            // | -x  (gl_only)  | gl     | gl       |  (one build, no regular)
            let (gl_out, nodes, final_num_org) = if !args.build_nodes {
                (None, zdbsp_lib::nodebuild::extract::NodeOutput::default(), 0u32)
            } else if args.conform_nodes && args.build_gl_nodes {
                let (starts, anchors) = if args.check_polyobjs {
                    collect_poly_spots(&processor.level)
                } else {
                    (Vec::new(), Vec::new())
                };
                let mut nb = NodeBuilder::new(&mut processor.level, starts, anchors, &map_name, true);
                nb.options = build_opts;
                nb.build();
                let num_org = nb.initial_vertex_count() as u32;
                // Order matters — extract_nodes() relies on `stored_seg` being unset,
                // which extract_gl() writes to.
                let reg = nb.extract_nodes();
                let gl = nb.extract_gl();
                processor.level.vertices = reg.vertices.clone();
                (Some(gl), reg, num_org)
            } else if args.gl_only && args.build_gl_nodes {
                let (starts, anchors) = if args.check_polyobjs {
                    collect_poly_spots(&processor.level)
                } else {
                    (Vec::new(), Vec::new())
                };
                let mut nb = NodeBuilder::new(&mut processor.level, starts, anchors, &map_name, true);
                nb.options = build_opts;
                nb.build();
                let num_org = nb.initial_vertex_count() as u32;
                let gl = nb.extract_gl();
                processor.level.vertices = gl.vertices.clone();
                (Some(gl), zdbsp_lib::nodebuild::extract::NodeOutput::default(), num_org)
            } else if args.build_gl_nodes {
                // -g: two builds. GL build → regular build (fresh reload).
                let (gl_out, _gl_num_org) = {
                    let (starts, anchors) = if args.check_polyobjs {
                        collect_poly_spots(&processor.level)
                    } else {
                        (Vec::new(), Vec::new())
                    };
                    let mut nb = NodeBuilder::new(&mut processor.level, starts, anchors, &map_name, true);
                    nb.options = build_opts;
                    nb.build();
                    let n = nb.initial_vertex_count() as u32;
                    (nb.extract_gl(), n)
                };
                processor = Processor::load(&mut reader, lump, args.no_prune)
                    .map_err(|e| format!("reload {map_name}: {e}"))?;
                let (starts, anchors) = if args.check_polyobjs {
                    collect_poly_spots(&processor.level)
                } else {
                    (Vec::new(), Vec::new())
                };
                let mut nb = NodeBuilder::new(&mut processor.level, starts, anchors, &map_name, false);
                nb.options = build_opts;
                nb.build();
                let num_org = nb.initial_vertex_count() as u32;
                let reg = nb.extract_nodes();
                processor.level.vertices = reg.vertices.clone();
                (Some(gl_out), reg, num_org)
            } else {
                let (starts, anchors) = if args.check_polyobjs {
                    collect_poly_spots(&processor.level)
                } else {
                    (Vec::new(), Vec::new())
                };
                let mut nb = NodeBuilder::new(&mut processor.level, starts, anchors, &map_name, false);
                nb.options = build_opts;
                nb.build();
                let num_org = nb.initial_vertex_count() as u32;
                let reg = nb.extract_nodes();
                processor.level.vertices = reg.vertices.clone();
                (None, reg, num_org)
            };

            writer::write_map(
                &mut out,
                &mut reader,
                lump,
                &processor.level,
                &nodes,
                gl_out.as_ref(),
                final_num_org,
                processor.format,
                opts,
            )
            .map_err(|e| format!("write {map_name}: {e}"))?;
            lump = reader.lump_after_map(lump);
        } else if reader.is_gl_nodes(lump) {
            // If we're rebuilding the map this GL block belongs to, drop the input GL.
            let gl_map_name: String = reader.lump_name(lump).chars().skip(3).collect();
            let rebuilding =
                args.build_nodes && args.map_filter.as_deref().map_or(true, |m| m.eq_ignore_ascii_case(&gl_map_name));
            if rebuilding {
                lump = reader.skip_gl_nodes(lump);
            } else {
                out.copy_lump(&mut reader, lump).map_err(|e| format!("copy gl: {e}"))?;
                lump += 1;
            }
        } else {
            out.copy_lump(&mut reader, lump).map_err(|e| format!("copy {lump}: {e}"))?;
            lump += 1;
        }
    }
    out.close().map_err(|e| format!("close output: {e}"))?;
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("zdbsp: {e}");
            eprintln!("Try `zdbsp --help' for more information.");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("zdbsp: {e}");
            ExitCode::FAILURE
        }
    }
}
