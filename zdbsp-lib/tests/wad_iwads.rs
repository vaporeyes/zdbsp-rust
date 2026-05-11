// ABOUTME: Integration test that opens the IWADs at ~/media/doom_wads/ and sanity-checks
// ABOUTME: the WAD reader's directory parsing and map-detection logic against real data.

use std::path::PathBuf;
use zdbsp_lib::wad::WadReader;

const CORPUS: &str = "/Users/jsh/media/doom_wads";

struct Expect {
    file: &'static str,
    is_iwad: bool,
    first_map: &'static str,
}

const EXPECTED: &[Expect] = &[
    Expect { file: "doom.wad",     is_iwad: true,  first_map: "E1M1" },
    Expect { file: "doom2.wad",    is_iwad: true,  first_map: "MAP01" },
    Expect { file: "tnt.wad",      is_iwad: true,  first_map: "MAP01" },
    Expect { file: "plutonia.wad", is_iwad: true,  first_map: "MAP01" },
    Expect { file: "heretic.wad",  is_iwad: true,  first_map: "E1M1" },
    Expect { file: "hexen.wad",    is_iwad: true,  first_map: "MAP01" },
    Expect { file: "hexdd.wad",    is_iwad: true,  first_map: "MAP33" },
];

#[test]
fn read_iwad_directories() {
    let corpus = PathBuf::from(CORPUS);
    if !corpus.is_dir() {
        eprintln!("SKIP: corpus dir {corpus:?} not present");
        return;
    }

    let mut checked = 0usize;
    for e in EXPECTED {
        let path = corpus.join(e.file);
        if !path.is_file() {
            eprintln!("SKIP {}: not found", e.file);
            continue;
        }
        let reader = WadReader::open(&path).expect("open wad");
        assert_eq!(reader.is_iwad(), e.is_iwad, "IWAD flag for {}", e.file);
        let first = reader.next_map(-1);
        assert!(first >= 0, "{} has no maps", e.file);
        let first_name = reader.lump_name(first).into_owned();
        assert_eq!(first_name, e.first_map, "first map of {}", e.file);
        checked += 1;
    }
    eprintln!("checked {checked} iwad(s)");
    assert!(checked > 0, "no expected IWADs were found");
}

#[test]
fn doom2_has_32_maps() {
    let path = PathBuf::from(CORPUS).join("doom2.wad");
    if !path.is_file() {
        eprintln!("SKIP: {path:?} not present");
        return;
    }
    let reader = WadReader::open(&path).unwrap();
    let mut count = 0usize;
    let mut idx = reader.next_map(-1);
    while idx >= 0 {
        count += 1;
        idx = reader.next_map(idx);
    }
    assert_eq!(count, 32, "doom2.wad should contain 32 maps");
}

#[test]
fn hexen_maps_have_behavior() {
    let path = PathBuf::from(CORPUS).join("hexen.wad");
    if !path.is_file() {
        eprintln!("SKIP: {path:?} not present");
        return;
    }
    let reader = WadReader::open(&path).unwrap();
    let first = reader.next_map(-1);
    assert!(first >= 0);
    assert!(reader.map_has_behavior(first), "hexen maps must have a BEHAVIOR lump");
}
