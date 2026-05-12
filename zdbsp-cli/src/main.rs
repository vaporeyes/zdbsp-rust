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
    compress_nodes: bool,
    compress_gl_nodes: bool,
    reject_mode: RejectMode,
    blockmap_mode: BlockmapMode,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        output: PathBuf::from("tmp.wad"),
        build_nodes: true,
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
            a.build_gl_nodes = true;
        } else if arg == "-z" || arg == "--compress" {
            a.compress_nodes = true;
        } else if arg == "-Z" || arg == "--compress-normal" {
            a.compress_gl_nodes = true;
        } else if arg == "-q" || arg == "--no-prune" {
            a.no_prune = true;
        } else if arg == "-r" || arg == "--empty-reject" {
            a.reject_mode = RejectMode::Create0;
        } else if arg == "-R" || arg == "--zero-reject" {
            a.reject_mode = RejectMode::CreateZeroes;
        } else if arg == "-b" || arg == "--empty-blockmap" {
            a.blockmap_mode = BlockmapMode::Create0;
        } else if arg == "--version" || arg == "-V" {
            println!("ZDBSP-rust 0.1.0");
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
            if reader.is_udmf(lump) {
                eprintln!("Skipping {map_name}: UDMF not supported in this build");
                // Copy the map block through unchanged.
                let after = reader.lump_after_map(lump);
                for l in lump..after {
                    out.copy_lump(&mut reader, l).map_err(|e| format!("copy {l}: {e}"))?;
                }
                lump = after;
                continue;
            }
            eprintln!("----{map_name}----");
            let mut processor = Processor::load(&mut reader, lump, args.no_prune)
                .map_err(|e| format!("load {map_name}: {e}"))?;

            // GL build (optional). Mirrors processor.cpp:617-636: build GL first, then
            // throw away the builder and load again for the regular build.
            let (gl_out, gl_num_org) = if args.build_nodes && args.build_gl_nodes {
                let (starts, anchors) = collect_poly_spots(&processor.level);
                let mut nb = NodeBuilder::new(&mut processor.level, starts, anchors, &map_name, true);
                nb.build();
                let num_org = nb.initial_vertex_count() as u32;
                let extracted = nb.extract_gl();
                (Some(extracted), num_org)
            } else {
                (None, 0)
            };

            // Reload the level for the regular build so the vertex array starts fresh.
            // (The GL build mutated processor.level.lines to reference the GL builder's
            // vertex indices.)
            if args.build_gl_nodes && args.build_nodes {
                processor = Processor::load(&mut reader, lump, args.no_prune)
                    .map_err(|e| format!("reload {map_name}: {e}"))?;
            }

            let (nodes, num_org_verts) = if args.build_nodes {
                let (starts, anchors) = collect_poly_spots(&processor.level);
                let mut nb = NodeBuilder::new(&mut processor.level, starts, anchors, &map_name, false);
                nb.build();
                let num_org = nb.initial_vertex_count() as u32;
                let out = nb.extract_nodes();
                // Mirror processor.cpp:598-599: replace Level.Vertices with the builder's
                // expanded array so the blockmap rebuild reads the right coords.
                processor.level.vertices = out.vertices.clone();
                (out, num_org)
            } else {
                (zdbsp_lib::nodebuild::extract::NodeOutput::default(), 0)
            };

            let final_num_org = if gl_out.is_some() { gl_num_org } else { num_org_verts };

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
