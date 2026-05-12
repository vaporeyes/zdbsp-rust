// ABOUTME: UDMF roundtrip tests. Builds a synthetic UDMF wad in-memory, parses it
// ABOUTME: via Processor::load, and verifies the level fields are populated correctly.

use zdbsp_lib::processor::{MapFormat, Processor};
use zdbsp_lib::wad::{WadReader, WadWriter};

const TEXTMAP: &str = r#"namespace = "ZDoom";

thing
{
x = 32.0;
y = 16.0;
angle = 90;
type = 1;
skill1 = true;
}

vertex
{
x = 0.0;
y = 0.0;
}

vertex
{
x = 256.0;
y = 0.0;
}

vertex
{
x = 256.0;
y = 256.0;
}

vertex
{
x = 0.0;
y = 256.0;
}

linedef
{
v1 = 0;
v2 = 1;
sidefront = 0;
}

linedef
{
v1 = 1;
v2 = 2;
sidefront = 1;
}

linedef
{
v1 = 2;
v2 = 3;
sidefront = 2;
}

linedef
{
v1 = 3;
v2 = 0;
sidefront = 3;
}

sidedef
{
sector = 0;
texturemiddle = "STARTAN";
}

sidedef
{
sector = 0;
texturemiddle = "STARTAN";
}

sidedef
{
sector = 0;
texturemiddle = "STARTAN";
}

sidedef
{
sector = 0;
texturemiddle = "STARTAN";
}

sector
{
heightfloor = 0;
heightceiling = 128;
texturefloor = "FLAT5";
textureceiling = "F_SKY1";
lightlevel = 200;
}
"#;

fn build_udmf_wad() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("zdbsp-rust-udmf-rt");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("udmf.wad");
    let mut w = WadWriter::create(&path, false).unwrap();
    w.create_label("MAP01").unwrap();
    w.write_lump("TEXTMAP", TEXTMAP.as_bytes()).unwrap();
    w.create_label("ENDMAP").unwrap();
    w.close().unwrap();
    path
}

#[test]
fn detect_udmf() {
    let path = build_udmf_wad();
    let wad = WadReader::open(&path).unwrap();
    let idx = wad.next_map(-1);
    assert!(idx >= 0);
    assert!(wad.is_udmf(idx));
}

#[test]
fn load_udmf_populates_level() {
    let path = build_udmf_wad();
    let mut wad = WadReader::open(&path).unwrap();
    let idx = wad.next_map(-1);
    let p = Processor::load(&mut wad, idx, false).expect("load udmf");
    assert_eq!(p.format, MapFormat::Udmf);
    assert_eq!(p.level.num_things(), 1);
    assert_eq!(p.level.num_vertices(), 4);
    assert_eq!(p.level.num_lines(), 4);
    assert_eq!(p.level.num_sides(), 4);
    assert_eq!(p.level.num_sectors(), 1);
    // Vertex 1 should be at (256, 0) = (256<<16, 0).
    assert_eq!(p.level.vertices[1].x, 256 << 16);
    assert_eq!(p.level.vertices[1].y, 0);
    // Thing at (32, 16).
    assert_eq!(p.level.things[0].x, 32 << 16);
    assert_eq!(p.level.things[0].y, 16 << 16);
    assert_eq!(p.level.things[0].angle, 90);
    assert_eq!(p.level.things[0].kind, 1);
    // Sidedef sector = 0.
    assert_eq!(p.level.sides[0].sector, 0);
    // Linedef v1, v2 plumbing.
    assert_eq!(p.level.lines[0].v1, 0);
    assert_eq!(p.level.lines[0].v2, 1);
    assert_eq!(p.level.lines[0].sidenum[0], 0);
    // The "extended" namespace check populates props on the level.
    assert!(p.level.props.iter().any(|k| k.key.eq_ignore_ascii_case("namespace")));
}

#[test]
fn parse_then_write_then_parse_is_idempotent() {
    use zdbsp_lib::udmf;
    let path = build_udmf_wad();
    let mut wad = WadReader::open(&path).unwrap();
    let idx = wad.next_map(-1);
    let p = Processor::load(&mut wad, idx, false).unwrap();

    // Round-trip the TEXTMAP through our writer and re-parse.
    let out_path = std::env::temp_dir().join("zdbsp-rust-udmf-rt/round.wad");
    let mut out = WadWriter::create(&out_path, false).unwrap();
    out.create_label("MAP01").unwrap();
    udmf::write_text_map(&mut out, &p.level, false).unwrap();
    out.create_label("ENDMAP").unwrap();
    out.close().unwrap();

    let mut wad2 = WadReader::open(&out_path).unwrap();
    let idx2 = wad2.next_map(-1);
    let p2 = Processor::load(&mut wad2, idx2, false).unwrap();
    assert_eq!(p2.level.num_things(), p.level.num_things());
    assert_eq!(p2.level.num_lines(), p.level.num_lines());
    assert_eq!(p2.level.num_sides(), p.level.num_sides());
    assert_eq!(p2.level.num_sectors(), p.level.num_sectors());
    assert_eq!(p2.level.num_vertices(), p.level.num_vertices());
    // Vertex coords should survive a round trip exactly.
    for (a, b) in p.level.vertices.iter().zip(p2.level.vertices.iter()) {
        assert_eq!(a.x, b.x);
        assert_eq!(a.y, b.y);
    }
}
