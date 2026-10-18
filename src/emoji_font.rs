//! A colour bitmap font made at runtime, so Discord's custom emoji show inline
//! in message text: Slint's `StyledText` draws only glyphs. Each emoji is a PNG
//! in a CBDT table, the format of the Noto Color Emoji build Yapfast already draws,
//! at a Private Use Area code point. On macOS, where Skia reads fonts through
//! Core Text, which doesn't take CBDT, the PNGs go in an sbix table, Apple's
//! format and Apple Color Emoji's. The font's vertical metrics are the UI
//! typeface's, Atkinson Hyperlegible Next's, so a line with an emoji keeps its
//! baseline.

/// Font units per em, as in Atkinson Hyperlegible Next and Satoshi.
const UNITS_PER_EM: u16 = 1000;
/// Atkinson Hyperlegible Next's ascender, descender and line gap, in font
/// units. Satoshi's, the second choice of typeface, are 1010, -240 and 100.
const ASCENDER: i16 = 984;
const DESCENDER: i16 = -316;
const LINE_GAP: i16 = 0;
/// The pixels per em of the bitmaps. Text draws them scaled to its size.
const PPEM: u8 = 100;
/// The side of each emoji's square bitmap: 1.36 em, Discord's 22 pixels in 16
/// pixel text. It spans the line, its foot just below the descenders.
pub const SIDE: u32 = 136;
const BEARING_X: i8 = 2;
/// The bitmap's top, in pixels above the baseline. With Satoshi's shallower
/// descender it was 107.
const BEARING_Y: i8 = 102;
const ADVANCE: u8 = 140;

/// A font with each PNG of `glyphs`, which must be [`SIDE`] pixels square, at
/// its code point. Glyph 0 is the empty `.notdef`.
pub fn build(family: &str, glyphs: &[(char, Vec<u8>)]) -> Vec<u8> {
    let count = u16::try_from(glyphs.len() + 1).expect("fewer than 65535 glyphs");
    let advance = units(i32::from(ADVANCE));
    let bitmaps = if cfg!(target_os = "macos") {
        vec![
            (*b"sbix", sbix(glyphs)),
            (*b"glyf", glyf(glyphs)),
            (*b"loca", loca(glyphs)),
        ]
    } else {
        vec![(*b"CBDT", cbdt(glyphs)), (*b"CBLC", cblc(glyphs))]
    };
    let mut tables: Vec<([u8; 4], Vec<u8>)> = bitmaps;
    tables.extend([
        (*b"OS/2", os2(advance, glyphs)),
        (*b"cmap", cmap(glyphs)),
        (*b"head", head(advance)),
        (*b"hhea", hhea(advance, count)),
        (*b"hmtx", hmtx(advance, count)),
        (*b"maxp", maxp(count)),
        (*b"name", name(family)),
        (*b"post", post()),
    ]);
    // In the order of their tags, as fonts list their tables.
    tables.sort_by_key(|(tag, _)| *tag);
    let mut font = Vec::new();
    let entry_selector = (tables.len() as u16).ilog2() as u16;
    let search_range = 16u16 << entry_selector;
    put32(&mut font, 0x0001_0000);
    put16(&mut font, tables.len() as u16);
    put16(&mut font, search_range);
    put16(&mut font, entry_selector);
    put16(&mut font, tables.len() as u16 * 16 - search_range);
    let mut offset = 12 + 16 * tables.len();
    let mut head_at = 0;
    for (tag, data) in &tables {
        if tag == b"head" {
            head_at = offset;
        }
        font.extend_from_slice(tag);
        put32(&mut font, checksum(data));
        put32(&mut font, offset as u32);
        put32(&mut font, data.len() as u32);
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    // The whole font sums to 0xB1B0AFBA.
    let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&font));
    font[head_at + 8..head_at + 12].copy_from_slice(&adjustment.to_be_bytes());
    font
}

/// Pixels of the bitmaps in font units.
fn units(pixels: i32) -> i16 {
    i16::try_from(pixels * i32::from(UNITS_PER_EM) / i32::from(PPEM)).expect("fits a font unit")
}

fn put8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// The bitmaps: each a small metrics record and its PNG (glyph format 17).
fn cbdt(glyphs: &[(char, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    put16(&mut out, 3);
    put16(&mut out, 0);
    for (_, png) in glyphs {
        put8(&mut out, SIDE as u8);
        put8(&mut out, SIDE as u8);
        put8(&mut out, BEARING_X as u8);
        put8(&mut out, BEARING_Y as u8);
        put8(&mut out, ADVANCE);
        put32(&mut out, png.len() as u32);
        out.extend_from_slice(png);
    }
    out
}

/// The bitmaps as Apple has them: one strike at [`PPEM`], each glyph's PNG at
/// the bottom left corner of its box in `glyf`. Glyph 0, `.notdef`, has none.
fn sbix(glyphs: &[(char, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    put16(&mut out, 1);
    // Bit 0 is always set.
    put16(&mut out, 1);
    put32(&mut out, 1);
    // The strike, after the header.
    put32(&mut out, 12);
    put16(&mut out, u16::from(PPEM));
    put16(&mut out, 72);
    // Each glyph's offset from the strike's start, and where the last ends.
    let mut offset = 4 + 4 * (glyphs.len() as u32 + 2);
    put32(&mut out, offset);
    put32(&mut out, offset);
    for (_, png) in glyphs {
        offset += 8 + png.len() as u32;
        put32(&mut out, offset);
    }
    for (_, png) in glyphs {
        put16(&mut out, 0);
        put16(&mut out, 0);
        out.extend_from_slice(b"png ");
        out.extend_from_slice(png);
    }
    out
}

/// The bytes of each emoji's outline in `glyf`.
const OUTLINE: usize = 26;

/// Core Text takes a font only with outlines, and draws an sbix bitmap over
/// its glyph's box, which an empty outline doesn't have. As in Apple Color
/// Emoji, each emoji's outline is two contours of a point each, at the
/// corners of where its bitmap goes.
fn glyf(glyphs: &[(char, Vec<u8>)]) -> Vec<u8> {
    let left = units(i32::from(BEARING_X));
    let bottom = units(i32::from(BEARING_Y) - SIDE as i32);
    let right = units(i32::from(BEARING_X) + SIDE as i32);
    let top = units(i32::from(BEARING_Y));
    let mut out = Vec::new();
    for _ in glyphs {
        put16(&mut out, 2);
        for edge in [left, bottom, right, top] {
            put16(&mut out, edge as u16);
        }
        // The contours' last points, and no instructions.
        put16(&mut out, 0);
        put16(&mut out, 1);
        put16(&mut out, 0);
        // Both points on the curve, their coordinates 16-bit steps from the last.
        put8(&mut out, 1);
        put8(&mut out, 1);
        put16(&mut out, left as u16);
        put16(&mut out, (right - left) as u16);
        put16(&mut out, bottom as u16);
        put16(&mut out, (top - bottom) as u16);
    }
    debug_assert_eq!(out.len(), OUTLINE * glyphs.len());
    out
}

/// Where each glyph's outline is in `glyf`, in halves of its offset, as
/// `head` says: `.notdef`'s empty, then each emoji's.
fn loca(glyphs: &[(char, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    put16(&mut out, 0);
    for index in 0..=glyphs.len() {
        put16(&mut out, (index * OUTLINE / 2) as u16);
    }
    out
}

/// Where each glyph's bitmap is in CBDT: one strike, one index of offsets.
fn cblc(glyphs: &[(char, Vec<u8>)]) -> Vec<u8> {
    const SIZE_RECORD: u32 = 48;
    let count = glyphs.len();
    let mut out = Vec::new();
    put16(&mut out, 3);
    put16(&mut out, 0);
    put32(&mut out, 1);
    // The strike: its index array follows this record.
    let index_tables = 8 + 8 + 4 * (count as u32 + 1);
    put32(&mut out, 8 + SIZE_RECORD);
    put32(&mut out, index_tables);
    put32(&mut out, 1);
    put32(&mut out, 0);
    for _ in 0..2 {
        let pixels =
            |units: i16| (i32::from(units) * i32::from(PPEM) / i32::from(UNITS_PER_EM)) as i8;
        let line: [i8; 12] = [
            pixels(ASCENDER),
            pixels(DESCENDER),
            ADVANCE as i8,
            0,
            0,
            0,
            BEARING_X,
            BEARING_X,
            BEARING_Y,
            (i32::from(BEARING_Y) - SIDE as i32) as i8,
            0,
            0,
        ];
        out.extend(line.map(|value| value as u8));
    }
    put16(&mut out, 1);
    put16(&mut out, count as u16);
    put8(&mut out, PPEM);
    put8(&mut out, PPEM);
    put8(&mut out, 32);
    // Horizontal metrics.
    put8(&mut out, 1);
    // The index array, and its one index: format 1, of PNGs at offsets in CBDT.
    put16(&mut out, 1);
    put16(&mut out, count as u16);
    put32(&mut out, 8);
    put16(&mut out, 1);
    put16(&mut out, 17);
    put32(&mut out, 4);
    // Each glyph's offset into CBDT's data after its header, and where the last ends.
    let mut offset = 0u32;
    put32(&mut out, offset);
    for (_, png) in glyphs {
        offset += 9 + png.len() as u32;
        put32(&mut out, offset);
    }
    out
}

fn os2(advance: i16, glyphs: &[(char, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    put16(&mut out, 4);
    put16(&mut out, advance as u16);
    put16(&mut out, 400);
    put16(&mut out, 5);
    put16(&mut out, 0);
    // Subscript, superscript and strikeout.
    for value in [650, 600, 0, 75, 650, 600, 0, 350, 50, 300] {
        put16(&mut out, value);
    }
    put16(&mut out, 0);
    out.extend_from_slice(&[0; 10]);
    // Unicode ranges: the Private Use Areas, bits 60 and 90.
    put32(&mut out, 0);
    put32(&mut out, 1 << (60 - 32));
    put32(&mut out, 1 << (90 - 64));
    put32(&mut out, 0);
    out.extend_from_slice(b"YAPF");
    // Regular, and the typographic metrics are the ones to use.
    put16(&mut out, 0x0040 | 0x0080);
    let first = glyphs
        .first()
        .map_or(0xFFFF, |(c, _)| (*c as u32).min(0xFFFF) as u16);
    put16(&mut out, first);
    put16(&mut out, 0xFFFF);
    put16(&mut out, ASCENDER as u16);
    put16(&mut out, DESCENDER as u16);
    put16(&mut out, LINE_GAP as u16);
    put16(&mut out, ASCENDER as u16);
    put16(&mut out, (-DESCENDER) as u16);
    put32(&mut out, 0);
    put32(&mut out, 0);
    put16(&mut out, 500);
    put16(&mut out, 700);
    put16(&mut out, 0);
    put16(&mut out, 32);
    put16(&mut out, 1);
    out
}

/// Code points to glyphs, for Unicode and for Windows, in one format 12 subtable.
fn cmap(glyphs: &[(char, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    put16(&mut out, 0);
    put16(&mut out, 2);
    for (platform, encoding) in [(0, 4), (3, 10)] {
        put16(&mut out, platform);
        put16(&mut out, encoding);
        put32(&mut out, 4 + 2 * 8);
    }
    let mut groups: Vec<(u32, u32, u32)> = glyphs
        .iter()
        .enumerate()
        .map(|(index, (c, _))| (*c as u32, *c as u32, index as u32 + 1))
        .collect();
    groups.sort_unstable();
    put16(&mut out, 12);
    put16(&mut out, 0);
    put32(&mut out, 16 + 12 * groups.len() as u32);
    put32(&mut out, 0);
    put32(&mut out, groups.len() as u32);
    for (start, end, glyph) in groups {
        put32(&mut out, start);
        put32(&mut out, end);
        put32(&mut out, glyph);
    }
    out
}

fn head(advance: i16) -> Vec<u8> {
    let mut out = Vec::new();
    put32(&mut out, 0x0001_0000);
    put32(&mut out, 0x0001_0000);
    // The checksum adjustment, which `build` sets last.
    put32(&mut out, 0);
    put32(&mut out, 0x5F0F_3CF5);
    // The baseline is at y 0, and the left side bearing at x 0.
    put16(&mut out, 0b11);
    put16(&mut out, UNITS_PER_EM);
    out.extend_from_slice(&[0; 16]);
    put16(&mut out, units(i32::from(BEARING_X)) as u16);
    put16(&mut out, units(i32::from(BEARING_Y) - SIDE as i32) as u16);
    put16(&mut out, advance as u16);
    put16(&mut out, units(i32::from(BEARING_Y)) as u16);
    put16(&mut out, 0);
    put16(&mut out, 8);
    put16(&mut out, 2);
    put16(&mut out, 0);
    put16(&mut out, 0);
    out
}

fn hhea(advance: i16, count: u16) -> Vec<u8> {
    let mut out = Vec::new();
    put32(&mut out, 0x0001_0000);
    put16(&mut out, ASCENDER as u16);
    put16(&mut out, DESCENDER as u16);
    put16(&mut out, LINE_GAP as u16);
    put16(&mut out, advance as u16);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, advance as u16);
    put16(&mut out, 1);
    put16(&mut out, 0);
    put16(&mut out, 0);
    out.extend_from_slice(&[0; 8]);
    put16(&mut out, 0);
    put16(&mut out, count);
    out
}

fn hmtx(advance: i16, count: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let bearing = units(i32::from(BEARING_X));
    for _ in 0..count {
        put16(&mut out, advance as u16);
        put16(&mut out, bearing as u16);
    }
    out
}

/// Version 0.5: the glyph count alone, for a font without outlines, or on
/// macOS 1.0, for the outlines `glyf` gives each emoji.
fn maxp(count: u16) -> Vec<u8> {
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        put32(&mut out, 0x0001_0000);
        put16(&mut out, count);
        // Each outline's two points in two contours; no composites.
        put16(&mut out, 2);
        put16(&mut out, 2);
        put16(&mut out, 0);
        put16(&mut out, 0);
        // Two zones, and no hinting.
        put16(&mut out, 2);
        out.extend_from_slice(&[0; 16]);
    } else {
        put32(&mut out, 0x0000_5000);
        put16(&mut out, count);
    }
    out
}

fn name(family: &str) -> Vec<u8> {
    let records = [
        (1, family.to_owned()),
        (2, "Regular".to_owned()),
        (4, family.to_owned()),
        (6, family.replace(' ', "")),
    ];
    let strings: Vec<Vec<u8>> = records
        .iter()
        .map(|(_, text)| text.encode_utf16().flat_map(u16::to_be_bytes).collect())
        .collect();
    let mut out = Vec::new();
    put16(&mut out, 0);
    put16(&mut out, records.len() as u16);
    put16(&mut out, 6 + 12 * records.len() as u16);
    let mut offset = 0;
    for ((id, _), string) in records.iter().zip(&strings) {
        put16(&mut out, 3);
        put16(&mut out, 1);
        put16(&mut out, 0x409);
        put16(&mut out, *id);
        put16(&mut out, string.len() as u16);
        put16(&mut out, offset);
        offset += string.len() as u16;
    }
    for string in strings {
        out.extend(string);
    }
    out
}

/// Version 3: no glyph names.
fn post() -> Vec<u8> {
    let mut out = Vec::new();
    put32(&mut out, 0x0003_0000);
    put32(&mut out, 0);
    put16(&mut out, (-100i16) as u16);
    put16(&mut out, 50);
    out.extend_from_slice(&[0; 20]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of `font`, by its tag.
    fn table<'a>(font: &'a [u8], tag: &[u8; 4]) -> &'a [u8] {
        let count = u16::from_be_bytes([font[4], font[5]]) as usize;
        let record = (0..count)
            .map(|index| &font[12 + 16 * index..28 + 16 * index])
            .find(|record| &record[..4] == tag)
            .expect("the table is there");
        let at = |range: std::ops::Range<usize>| {
            u32::from_be_bytes(record[range].try_into().unwrap()) as usize
        };
        &font[at(8..12)..at(8..12) + at(12..16)]
    }

    #[test]
    fn spaces_emoji_as_wide_as_they_are_drawn() {
        let font = build("Test", &[('\u{100000}', vec![1, 2, 3])]);
        let hmtx = table(&font, b"hmtx");
        // 140 pixels at 100 per em: 1.4 em, then 0.02 em before the picture.
        assert_eq!(hmtx[4..8], [0x05, 0x78, 0x00, 0x14]);
        let hhea = table(&font, b"hhea");
        assert_eq!(i16::from_be_bytes([hhea[4], hhea[5]]), ASCENDER);
        assert_eq!(
            checksum(&font),
            0xB1B0_AFBA,
            "the head checksum adjustment balances"
        );
    }
}
