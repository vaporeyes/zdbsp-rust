// ABOUTME: UDMF (Universal Doom Map Format) tokenizer, parser, and writer.
// ABOUTME: Port of processor_udmf.cpp + the minimal sc_man.cpp behavior it relies on.

use std::io;

use crate::fixed::Fixed;
use crate::level::{
    IntLineDef, IntSector, IntSideDef, IntThing, IntVertex, Level, MapSector, UdmfKey,
    WideVertex, NO_INDEX,
};
use crate::wad::WadWriter;

#[derive(Debug, thiserror::Error)]
pub enum UdmfError {
    #[error("UDMF parse error at line {line}: {msg}")]
    Parse { line: u32, msg: String },
    #[error("UDMF expected {expected}, got {got} at line {line}")]
    Unexpected { expected: String, got: String, line: u32 },
}

/// A token produced by the UDMF tokenizer. The raw text is preserved so that the
/// writer can re-emit values verbatim — matching the C++ behavior where unknown keys
/// keep their original textual form (including quotes for strings, signs for ints).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub line: u32,
    pub quoted: bool,
}

impl Token {
    pub fn parse_int(&self) -> Option<i32> {
        self.text.parse().ok()
    }
    pub fn parse_float(&self) -> Option<f64> {
        self.text.parse().ok()
    }
}

/// Streaming UDMF lexer.
pub struct Tokenizer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
}

impl<'a> Tokenizer<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0, line: 1 }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn skip_ws_and_comments(&mut self) {
        while let Some(c) = self.peek() {
            match c {
                b' ' | b'\t' | b'\r' | b'\n' => {
                    self.bump();
                }
                b'/' if self.src.get(self.pos + 1).copied() == Some(b'/') => {
                    // Line comment.
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                b'/' if self.src.get(self.pos + 1).copied() == Some(b'*') => {
                    // Block comment.
                    self.pos += 2;
                    while self.pos + 1 < self.src.len() {
                        if self.src[self.pos] == b'*' && self.src[self.pos + 1] == b'/' {
                            self.pos += 2;
                            break;
                        }
                        if self.src[self.pos] == b'\n' {
                            self.line += 1;
                        }
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    /// Read the next token, or `None` at EOF. Quoted strings include their surrounding
    /// quotes in the token text, matching `sc_String`'s contents in the C++ reference.
    pub fn next(&mut self) -> Option<Token> {
        self.skip_ws_and_comments();
        let start = self.pos;
        let line = self.line;
        let c = self.peek()?;

        if c == b'"' {
            // Quoted string — include quotes in text.
            self.bump();
            while let Some(ch) = self.peek() {
                if ch == b'"' {
                    self.bump();
                    break;
                }
                if ch == b'\\' {
                    self.bump();
                    if self.peek().is_some() {
                        self.bump();
                    }
                    continue;
                }
                self.bump();
            }
            let text = std::str::from_utf8(&self.src[start..self.pos])
                .unwrap_or("")
                .to_string();
            return Some(Token { text, line, quoted: true });
        }

        if is_ident_start(c) || c == b'-' || c == b'+' || c == b'.' || c.is_ascii_digit() {
            // Identifier or numeric literal — consume an unbroken run of word chars,
            // digits, dot, and sign chars after the first.
            self.bump();
            while let Some(ch) = self.peek() {
                if is_ident_continue(ch) || ch == b'.' || ch.is_ascii_digit() {
                    self.bump();
                } else {
                    break;
                }
            }
            let text = std::str::from_utf8(&self.src[start..self.pos])
                .unwrap_or("")
                .to_string();
            return Some(Token { text, line, quoted: false });
        }

        // Single-char punctuation.
        self.bump();
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .unwrap_or("")
            .to_string();
        Some(Token { text, line, quoted: false })
    }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_continue(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Helper that pulls a `key = value;` pair from the tokenizer. The value retains its
/// original token text (quotes preserved for strings, sign preserved for numbers).
fn parse_kv(tk: &mut Tokenizer) -> Result<(Token, Token), UdmfError> {
    let key = tk.next().ok_or_else(|| UdmfError::Parse {
        line: tk.line,
        msg: "unexpected eof reading key".into(),
    })?;
    expect_text(tk, "=")?;
    let value = tk.next().ok_or_else(|| UdmfError::Parse {
        line: tk.line,
        msg: "unexpected eof reading value".into(),
    })?;
    expect_text(tk, ";")?;
    Ok((key, value))
}

fn expect_text(tk: &mut Tokenizer, want: &str) -> Result<(), UdmfError> {
    let t = tk.next().ok_or_else(|| UdmfError::Unexpected {
        expected: want.into(),
        got: "<eof>".into(),
        line: tk.line,
    })?;
    if t.text == want {
        Ok(())
    } else {
        Err(UdmfError::Unexpected {
            expected: want.into(),
            got: t.text,
            line: t.line,
        })
    }
}

/// Parse a single fixed-point coord from a token. `xs_Fix<16>::ToFix(double)` from
/// the C++ rounds half-up; Rust's f64 → i32 truncates. For UDMF parity we want the
/// matching round-half-to-zero behavior. We replicate it via the same formula the
/// C++ helper expands to internally:  `(int)(val * 65536.0 + (val < 0 ? -0.5 : 0.5))`.
fn parse_fixed(tok: &Token) -> Result<Fixed, UdmfError> {
    let val: f64 = tok.parse_float().ok_or_else(|| UdmfError::Parse {
        line: tok.line,
        msg: format!("expected float, got {:?}", tok.text),
    })?;
    if !(-32768.0..=32767.0).contains(&val) {
        return Err(UdmfError::Parse {
            line: tok.line,
            msg: format!("fixed-point value {val} out of range"),
        });
    }
    let scaled = val * 65536.0;
    let rounded = if val < 0.0 { scaled - 0.5 } else { scaled + 0.5 };
    Ok(rounded as Fixed)
}

fn parse_int(tok: &Token) -> Result<i32, UdmfError> {
    tok.parse_int().ok_or_else(|| UdmfError::Parse {
        line: tok.line,
        msg: format!("expected int, got {:?}", tok.text),
    })
}

/// Indicates whether the loaded map uses an "extended" namespace (ZDoom / Hexen /
/// Vavoom) where line specials are first-class fields, not packed args.
pub struct ParseOutcome {
    pub extended: bool,
}

/// Parse a TEXTMAP lump into `level`. Returns the namespace's extended-ness, which the
/// caller uses to pick MapFormat.
pub fn parse_text_map(src: &[u8], level: &mut Level) -> Result<ParseOutcome, UdmfError> {
    let mut tk = Tokenizer::new(src);
    let mut extended = false;
    let mut vertices: Vec<WideVertex> = Vec::new();

    // Global keys come before the first block.
    loop {
        // Peek ahead: if next token is followed by `{`, it's a block name. If it's
        // followed by `=`, it's a global key. Lookahead via SavePos.
        let saved = tk.pos;
        let saved_line = tk.line;
        let Some(t1) = tk.next() else { break };
        let Some(t2) = tk.next() else { break };
        // Restore.
        tk.pos = saved;
        tk.line = saved_line;
        if t2.text == "=" {
            let (key, value) = parse_kv(&mut tk)?;
            if key.text.eq_ignore_ascii_case("namespace") {
                let v = value.text.trim_matches('"');
                if v.eq_ignore_ascii_case("ZDoom")
                    || v.eq_ignore_ascii_case("Hexen")
                    || v.eq_ignore_ascii_case("Vavoom")
                {
                    extended = true;
                }
            }
            level.props.push(UdmfKey { key: key.text, value: value.text });
        } else if t2.text == "{" {
            // Block — process below.
            let _ = t1;
            break;
        } else {
            return Err(UdmfError::Unexpected {
                expected: "= or {".into(),
                got: t2.text,
                line: t2.line,
            });
        }
    }

    // Block loop. Each block is `<kind> { kv* }`.
    while let Some(kind) = tk.next() {
        expect_text(&mut tk, "{")?;
        match kind.text.to_ascii_lowercase().as_str() {
            "thing" => {
                let mut t = IntThing::default();
                parse_block(&mut tk, |key, value| {
                    match key.text.to_ascii_lowercase().as_str() {
                        "x" => t.x = parse_fixed(value)?,
                        "y" => t.y = parse_fixed(value)?,
                        "angle" => t.angle = parse_int(value)? as i16,
                        "type" => t.kind = parse_int(value)? as i16,
                        _ => {}
                    }
                    t.props.push(UdmfKey { key: key.text.clone(), value: value.text.clone() });
                    Ok(())
                })?;
                level.things.push(t);
            }
            "linedef" => {
                let mut ld = IntLineDef {
                    v1: NO_INDEX,
                    v2: NO_INDEX,
                    sidenum: [NO_INDEX, NO_INDEX],
                    ..IntLineDef::default()
                };
                parse_block(&mut tk, |key, value| {
                    let k = key.text.to_ascii_lowercase();
                    match k.as_str() {
                        "v1" => {
                            ld.v1 = parse_int(value)? as u32;
                            return Ok(()); // do not store in props
                        }
                        "v2" => {
                            ld.v2 = parse_int(value)? as u32;
                            return Ok(());
                        }
                        "sidefront" => {
                            ld.sidenum[0] = parse_int(value)? as u32;
                            return Ok(());
                        }
                        "sideback" => {
                            ld.sidenum[1] = parse_int(value)? as u32;
                            return Ok(());
                        }
                        "special" if extended => {
                            ld.special = parse_int(value)?;
                        }
                        "arg0" if extended => {
                            ld.args[0] = parse_int(value)?;
                        }
                        _ => {}
                    }
                    ld.props.push(UdmfKey { key: key.text.clone(), value: value.text.clone() });
                    Ok(())
                })?;
                level.lines.push(ld);
            }
            "sidedef" => {
                let mut sd = IntSideDef::default();
                sd.sector = NO_INDEX;
                parse_block(&mut tk, |key, value| {
                    if key.text.eq_ignore_ascii_case("sector") {
                        sd.sector = parse_int(value)? as u32;
                        return Ok(());
                    }
                    sd.props.push(UdmfKey { key: key.text.clone(), value: value.text.clone() });
                    Ok(())
                })?;
                level.sides.push(sd);
            }
            "sector" => {
                let mut sec = IntSector {
                    data: MapSector::default(),
                    props: Vec::new(),
                };
                parse_block(&mut tk, |key, value| {
                    sec.props.push(UdmfKey { key: key.text.clone(), value: value.text.clone() });
                    Ok(())
                })?;
                level.sectors.push(sec);
            }
            "vertex" => {
                let mut vt = WideVertex::default();
                let mut vtp = IntVertex::default();
                parse_block(&mut tk, |key, value| {
                    match key.text.to_ascii_lowercase().as_str() {
                        "x" => vt.x = parse_fixed(value)?,
                        "y" => vt.y = parse_fixed(value)?,
                        _ => {}
                    }
                    vtp.props.push(UdmfKey { key: key.text.clone(), value: value.text.clone() });
                    Ok(())
                })?;
                vt.index = vertices.len() as i32 + 1; // matches C++ pre-push numbering
                vertices.push(vt);
                level.vertex_props.push(vtp);
            }
            other => {
                return Err(UdmfError::Parse {
                    line: kind.line,
                    msg: format!("unknown UDMF block kind: {other}"),
                });
            }
        }
    }

    level.vertices = vertices;
    Ok(ParseOutcome { extended })
}

fn parse_block<F>(tk: &mut Tokenizer, mut on_kv: F) -> Result<(), UdmfError>
where
    F: FnMut(&Token, &Token) -> Result<(), UdmfError>,
{
    loop {
        let saved = tk.pos;
        let saved_line = tk.line;
        let Some(peek) = tk.next() else {
            return Err(UdmfError::Parse {
                line: tk.line,
                msg: "unexpected eof inside block".into(),
            });
        };
        if peek.text == "}" {
            return Ok(());
        }
        tk.pos = saved;
        tk.line = saved_line;
        let (key, value) = parse_kv(tk)?;
        on_kv(&key, &value)?;
    }
}

// ---- Writer side (text emission only) -------------------------------------------

/// Write a `TEXTMAP` lump for `level`. Matches `FProcessor::WriteTextMap`. Sets
/// `write_comments` from the C++ `-c` flag.
pub fn write_text_map(
    out: &mut WadWriter,
    level: &Level,
    write_comments: bool,
) -> io::Result<()> {
    out.start_lump("TEXTMAP")?;
    write_props(out, &level.props)?;

    for (i, t) in level.things.iter().enumerate() {
        emit_block(out, "thing", i, write_comments, &t.props)?;
    }
    for i in 0..level.vertices.len() {
        let vt = &level.vertices[i];
        if vt.index <= 0 {
            return Err(io::Error::other("Invalid vertex data."));
        }
        let vp = &level.vertex_props[(vt.index - 1) as usize];
        emit_block(out, "vertex", i, write_comments, &vp.props)?;
    }
    for (i, ld) in level.lines.iter().enumerate() {
        out.add_to_lump(b"linedef")?;
        if write_comments {
            out.add_to_lump(format!(" // {i}").as_bytes())?;
        }
        out.add_to_lump(b"\n{\n")?;
        write_int_prop(out, "v1", ld.v1 as i32)?;
        write_int_prop(out, "v2", ld.v2 as i32)?;
        if ld.sidenum[0] != NO_INDEX {
            write_int_prop(out, "sidefront", ld.sidenum[0] as i32)?;
        }
        if ld.sidenum[1] != NO_INDEX {
            write_int_prop(out, "sideback", ld.sidenum[1] as i32)?;
        }
        write_props(out, &ld.props)?;
        out.add_to_lump(b"}\n\n")?;
    }
    for (i, sd) in level.sides.iter().enumerate() {
        out.add_to_lump(b"sidedef")?;
        if write_comments {
            out.add_to_lump(format!(" // {i}").as_bytes())?;
        }
        out.add_to_lump(b"\n{\n")?;
        write_int_prop(out, "sector", sd.sector as i32)?;
        write_props(out, &sd.props)?;
        out.add_to_lump(b"}\n\n")?;
    }
    for (i, sec) in level.sectors.iter().enumerate() {
        emit_block(out, "sector", i, write_comments, &sec.props)?;
    }
    Ok(())
}

fn emit_block(
    out: &mut WadWriter,
    kind: &str,
    idx: usize,
    write_comments: bool,
    props: &[UdmfKey],
) -> io::Result<()> {
    out.add_to_lump(kind.as_bytes())?;
    if write_comments {
        out.add_to_lump(format!(" // {idx}").as_bytes())?;
    }
    out.add_to_lump(b"\n{\n")?;
    write_props(out, props)?;
    out.add_to_lump(b"}\n\n")?;
    Ok(())
}

fn write_props(out: &mut WadWriter, props: &[UdmfKey]) -> io::Result<()> {
    for p in props {
        out.add_to_lump(p.key.as_bytes())?;
        out.add_to_lump(b" = ")?;
        out.add_to_lump(p.value.as_bytes())?;
        out.add_to_lump(b";\n")?;
    }
    Ok(())
}

fn write_int_prop(out: &mut WadWriter, key: &str, value: i32) -> io::Result<()> {
    out.add_to_lump(key.as_bytes())?;
    out.add_to_lump(b" = ")?;
    out.add_to_lump(format!("{value};\n").as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"namespace = "ZDoom";

thing
{
x = 32.500;
y = -16.000;
angle = 90;
type = 1;
skill1 = true;
}

vertex
{
x = 0.000;
y = 0.000;
}

vertex
{
x = 128.000;
y = 0.000;
}

linedef
{
v1 = 0;
v2 = 1;
sidefront = 0;
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
}
"#;

    #[test]
    fn parse_sample() {
        let mut level = Level::default();
        let outcome = parse_text_map(SAMPLE.as_bytes(), &mut level).unwrap();
        assert!(outcome.extended);
        assert_eq!(level.things.len(), 1);
        assert_eq!(level.things[0].kind, 1);
        assert_eq!(level.things[0].angle, 90);
        assert_eq!(level.vertices.len(), 2);
        assert_eq!(level.lines.len(), 1);
        assert_eq!(level.lines[0].v1, 0);
        assert_eq!(level.lines[0].v2, 1);
        assert_eq!(level.sides.len(), 1);
        assert_eq!(level.sides[0].sector, 0);
        assert_eq!(level.sectors.len(), 1);
    }

    #[test]
    fn parse_fixed_rounds_like_cpp() {
        // 1.5 → 0x00018000 (1.5 << 16)
        let t = Token { text: "1.5".into(), line: 1, quoted: false };
        assert_eq!(parse_fixed(&t).unwrap(), 0x18000);
        // -1.5 → -0x18000 with the round-half-away-from-zero rule
        let t = Token { text: "-1.5".into(), line: 1, quoted: false };
        assert_eq!(parse_fixed(&t).unwrap(), -0x18000);
        // 0 → 0
        let t = Token { text: "0".into(), line: 1, quoted: false };
        assert_eq!(parse_fixed(&t).unwrap(), 0);
    }

    #[test]
    fn tokenizer_handles_comments() {
        let src = b"// a comment\n  identifier /* block */ 42 \"string with spaces\" ;";
        let mut tk = Tokenizer::new(src);
        assert_eq!(tk.next().unwrap().text, "identifier");
        assert_eq!(tk.next().unwrap().text, "42");
        let s = tk.next().unwrap();
        assert!(s.quoted);
        assert_eq!(s.text, "\"string with spaces\"");
        assert_eq!(tk.next().unwrap().text, ";");
        assert!(tk.next().is_none());
    }
}
