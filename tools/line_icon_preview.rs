//! Host preview of the line icons in `src/ui/sicon.rs`. Not part of the Windows build.
//!
//! ```
//! rustc --edition 2021 -O tools/line_icon_preview.rs -o line_icon_preview
//! ./line_icon_preview line-icons.png
//! ```
//!
//! The sheet shows every icon at 1x for each DPI step in both themes, the 100% masters at 8x,
//! and the three button rows that use them (tab row, favourites window, pane header) at rest,
//! hover, and disabled, with the colours `draw_button` and `paint_icon` pick.

#![allow(dead_code)]

#[path = "../src/ui/sicon.rs"]
mod sicon;

use sicon::{LineIcon, line_icon_coverage, line_icon_px};

const GREEN: u32 = 0x1FA463;
const DISABLED_ALPHA: f32 = 0.4;

struct Theme {
    name: &'static str,
    bg: u32,
    card: u32,
    text: u32,
    sub: u32,
}

const THEMES: [Theme; 2] = [
    Theme { name: "Light", bg: 0xF3F1EC, card: 0xFFFFFF, text: 0x2A2825, sub: 0x7C776F },
    Theme { name: "Dark", bg: 0x1C1D20, card: 0x26282C, text: 0xE6E8EB, sub: 0x9AA0A8 },
];

const DPIS: [(i32, &str); 5] = [(96, "100%"), (120, "125%"), (144, "150%"), (168, "175%"), (192, "200%")];

const ALL: [LineIcon; 19] = [
    LineIcon::List,
    LineIcon::Layout,
    LineIcon::Save,
    LineIcon::Load,
    LineIcon::Star,
    LineIcon::Basket,
    LineIcon::Watch,
    LineIcon::Gear,
    LineIcon::Add,
    LineIcon::Group,
    LineIcon::Up,
    LineIcon::Down,
    LineIcon::Rename,
    LineIcon::Del,
    LineIcon::Back,
    LineIcon::Fwd,
    LineIcon::OpenIn,
    LineIcon::StarFilled,
    LineIcon::StarAdd,
];

/// The tab row, left to right, as (icon, button width in DIP). The layout button is wider.
const TOOLBAR: [(LineIcon, i32); 8] = [
    (LineIcon::List, 32),
    (LineIcon::Layout, 32),
    (LineIcon::Save, 32),
    (LineIcon::Load, 32),
    (LineIcon::Star, 32),
    (LineIcon::Basket, 32),
    (LineIcon::Watch, 32),
    (LineIcon::Gear, 32),
];
const FAVBAR: [LineIcon; 6] = [LineIcon::Add, LineIcon::Group, LineIcon::Up, LineIcon::Down, LineIcon::Rename, LineIcon::Del];
const HEADER: [LineIcon; 7] = [LineIcon::Back, LineIcon::Fwd, LineIcon::Up, LineIcon::OpenIn, LineIcon::Add, LineIcon::StarAdd, LineIcon::StarFilled];

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "line-icons.png".into());
    let png = render();
    std::fs::write(&path, &png).unwrap_or_else(|e| panic!("write {path}: {e}"));
    println!("wrote {path} ({} bytes)", png.len());
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Rest,
    Hover,
    Disabled,
}

fn render() -> Vec<u8> {
    let (margin, label_h, gap) = (20, 16, 12);
    let big = 8;
    let big_n = line_icon_px(96);
    let per_row = 7;
    let cell = |dpi: i32| line_icon_px(dpi) + 8;
    let atlas_w = 64 + ALL.len() as i32 * cell(192);
    let big_w = per_row * (big_n * big + gap);
    let bar_w = 72 + DPIS.len() as i32 * 0 + (TOOLBAR.len() as i32 * 32 + 16 + FAVBAR.len() as i32 * 24 + 16 + HEADER.len() as i32 * 24) * 2;
    let sheet_w = margin * 2 + atlas_w.max(big_w).max(bar_w);
    let atlas_h: i32 = label_h + DPIS.iter().map(|&(d, _)| cell(d) + 4).sum::<i32>() + gap;
    let big_rows = (ALL.len() as i32 + per_row - 1) / per_row;
    let big_h = label_h + big_rows * (big_n * big + gap) + gap;
    let bars_h: i32 = label_h + [96, 192].iter().map(|&d| label_h + 3 * (24 * d / 96 + 6)).sum::<i32>() + gap;
    let sheet_h = margin * 2 + label_h + THEMES.len() as i32 * (label_h + atlas_h + big_h + bars_h);
    let (w, h) = (sheet_w as u32, sheet_h as u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    fill(&mut rgba, w, 0, 0, w, h, 0xD9D6D0);

    let mut y = margin;
    for t in &THEMES {
        fill(&mut rgba, w, 0, y - 4, w, (label_h + atlas_h + big_h + bars_h) as u32, t.bg);
        text(&mut rgba, w, h, margin, y, t.name, t.text);
        y += label_h;

        text(&mut rgba, w, h, margin, y, "1x", t.sub);
        y += label_h;
        for &(dpi, name) in &DPIS {
            text(&mut rgba, w, h, margin, y + 4, name, t.sub);
            let mut x = margin + 64;
            for &kind in &ALL {
                let n = line_icon_px(dpi);
                blit_icon(&mut rgba, w, h, x + (cell(192) - n) / 2, y, n, dpi, 1, t.bg, t.sub, kind);
                x += cell(192);
            }
            y += cell(dpi) + 4;
        }
        y += gap;

        text(&mut rgba, w, h, margin, y, "100% x8", t.sub);
        y += label_h;
        for (i, &kind) in ALL.iter().enumerate() {
            let (col, row) = (i as i32 % per_row, i as i32 / per_row);
            let (x, iy) = (margin + col * (big_n * big + gap), y + row * (big_n * big + gap));
            blit_icon(&mut rgba, w, h, x, iy, big_n, 96, big, t.card, t.text, kind);
        }
        y += big_rows * (big_n * big + gap) + gap;

        text(&mut rgba, w, h, margin, y, "Toolbar  Favourites  Pane header", t.sub);
        y += label_h;
        for dpi in [96, 192] {
            let name = DPIS.iter().find(|d| d.0 == dpi).unwrap().1;
            text(&mut rgba, w, h, margin, y, name, t.sub);
            y += label_h;
            let s = |v: i32| v * dpi / 96;
            for state in [State::Rest, State::Hover, State::Disabled] {
                let tag = match state {
                    State::Rest => "Rest",
                    State::Hover => "Hover",
                    State::Disabled => "Disabled",
                };
                text(&mut rgba, w, h, margin, y + 4, tag, t.sub);
                let mut x = margin + 72;
                // Tab row: `draw_button` on the window ground, sub ink at rest, full ink on hover.
                for &(kind, bw) in &TOOLBAR {
                    let ink = match state {
                        State::Rest => t.sub,
                        State::Hover => t.text,
                        State::Disabled => blend(t.bg, t.sub, 0.45),
                    };
                    let ink = if state == State::Rest && kind == LineIcon::Save { GREEN } else { ink };
                    let bg = if state == State::Hover { blend(t.bg, t.text, 0.07) } else { t.bg };
                    paint_button(&mut rgba, w, h, x, y, s(bw), s(24), dpi, bg, ink, kind);
                    x += s(bw);
                }
                x += s(16);
                // Favourites window: icon buttons on the card, text ink.
                for &kind in &FAVBAR {
                    let ink = if state == State::Disabled { blend(t.card, t.text, DISABLED_ALPHA) } else { t.text };
                    let bg = if state == State::Hover { blend(t.card, t.text, 0.08) } else { t.card };
                    paint_button(&mut rgba, w, h, x, y, s(24), s(24), dpi, bg, ink, kind);
                    x += s(24);
                }
                x += s(16);
                for &kind in &HEADER {
                    let ink = match state {
                        State::Rest => t.sub,
                        State::Hover => t.text,
                        State::Disabled => blend(t.bg, t.sub, 0.45),
                    };
                    let bg = if state == State::Hover { blend(t.bg, t.text, 0.07) } else { t.bg };
                    paint_button(&mut rgba, w, h, x, y, s(24), s(22), dpi, bg, ink, kind);
                    x += s(24);
                }
                y += s(24) + 6;
            }
        }
        y += gap;
    }
    encode_png(w, h, &rgba)
}

fn paint_button(buf: &mut [u8], sw: u32, sh: u32, x: i32, y: i32, bw: i32, bh: i32, dpi: i32, bg: u32, ink: u32, kind: LineIcon) {
    fill(buf, sw, x, y, bw as u32, bh as u32, bg);
    let n = line_icon_px(dpi);
    blit_icon(buf, sw, sh, x + (bw - n) / 2, y + (bh - n) / 2, n, dpi, 1, bg, ink, kind);
}

fn blit_icon(buf: &mut [u8], sw: u32, sh: u32, x: i32, y: i32, n: i32, dpi: i32, zoom: i32, bg: u32, ink: u32, kind: LineIcon) {
    let cov = line_icon_coverage(n, dpi, kind);
    for py in 0..n {
        for px in 0..n {
            let c = blend(bg, ink, cov[(py * n + px) as usize] as f32 / 255.0);
            for dy in 0..zoom {
                for dx in 0..zoom {
                    put(buf, sw, sh, x + px * zoom + dx, y + py * zoom + dy, c);
                }
            }
        }
    }
}

fn fill(buf: &mut [u8], sw: u32, x: i32, y: i32, w: u32, h: u32, rgb: u32) {
    for row in 0..h as i32 {
        for col in 0..w as i32 {
            put(buf, sw, u32::MAX, x + col, y + row, rgb);
        }
    }
}

fn put(buf: &mut [u8], sw: u32, sh: u32, x: i32, y: i32, rgb: u32) {
    let sh = if sh == u32::MAX { (buf.len() / (sw as usize * 4)) as u32 } else { sh };
    if x < 0 || y < 0 || x as u32 >= sw || y as u32 >= sh {
        return;
    }
    let i = ((y as u32 * sw + x as u32) * 4) as usize;
    buf[i] = (rgb >> 16) as u8;
    buf[i + 1] = (rgb >> 8) as u8;
    buf[i + 2] = rgb as u8;
    buf[i + 3] = 255;
}

fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = ((a >> sh & 0xFF) as f32, (b >> sh & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << sh
    };
    ch(16) | ch(8) | ch(0)
}

fn text(buf: &mut [u8], sw: u32, sh: u32, x: i32, y: i32, s: &str, rgb: u32) {
    let mut cx = x;
    for ch in s.chars() {
        glyph(buf, sw, sh, cx, y, ch, rgb);
        cx += 6;
    }
}

fn glyph(buf: &mut [u8], sw: u32, sh: u32, x: i32, y: i32, ch: char, rgb: u32) {
    let rows = font(ch);
    for (row, bits) in rows.iter().enumerate() {
        for col in 0..5 {
            if bits & (1 << (4 - col)) != 0 {
                put(buf, sw, sh, x + col, y + row as i32, rgb);
            }
        }
    }
}

fn font(ch: char) -> [u8; 7] {
    match ch {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '%' => [0x19, 0x19, 0x02, 0x04, 0x08, 0x13, 0x13],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'i' => [0x04, 0x00, 0x0C, 0x04, 0x04, 0x04, 0x0E],
        'g' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x11, 0x0E],
        'h' => [0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x11],
        't' => [0x04, 0x04, 0x1F, 0x04, 0x04, 0x04, 0x03],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'a' => [0x00, 0x00, 0x0E, 0x01, 0x0F, 0x11, 0x0F],
        'r' => [0x00, 0x00, 0x16, 0x19, 0x10, 0x10, 0x10],
        'k' => [0x10, 0x10, 0x12, 0x14, 0x18, 0x14, 0x12],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'o' => [0x00, 0x00, 0x0E, 0x11, 0x11, 0x11, 0x0E],
        'v' => [0x00, 0x00, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'e' => [0x00, 0x00, 0x0E, 0x11, 0x1F, 0x10, 0x0E],
        's' => [0x00, 0x00, 0x0E, 0x10, 0x0E, 0x01, 0x1E],
        'b' => [0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x1E],
        'l' => [0x0C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'd' => [0x01, 0x01, 0x0D, 0x13, 0x11, 0x11, 0x0F],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'x' => [0x00, 0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'u' => [0x00, 0x00, 0x11, 0x11, 0x11, 0x13, 0x0D],
        'y' => [0x00, 0x00, 0x11, 0x11, 0x0F, 0x01, 0x0E],
        'c' => [0x00, 0x00, 0x0E, 0x10, 0x10, 0x10, 0x0E],
        'm' => [0x00, 0x00, 0x1A, 0x15, 0x15, 0x11, 0x11],
        'p' => [0x00, 0x00, 0x1E, 0x11, 0x1E, 0x10, 0x10],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'n' => [0x00, 0x00, 0x16, 0x19, 0x11, 0x11, 0x11],
        _ => [0, 0, 0, 0, 0, 0, 0],
    }
}

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((w * 4 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0);
        let start = (y * w * 4) as usize;
        raw.extend_from_slice(&rgba[start..start + (w * 4) as usize]);
    }
    let mut png = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_src = kind.to_vec();
    crc_src.extend_from_slice(data);
    out.extend(crc32(&crc_src).to_be_bytes());
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut i = 0;
    while i < data.len() {
        let n = (data.len() - i).min(65535);
        let last = i + n >= data.len();
        out.push(if last { 1 } else { 0 });
        out.extend((n as u16).to_le_bytes());
        out.extend((!(n as u16)).to_le_bytes());
        out.extend_from_slice(&data[i..i + n]);
        i += n;
    }
    out.extend(adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut s1, mut s2) = (1u32, 0u32);
    for &b in data {
        s1 = (s1 + b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    (s2 << 16) | s1
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = if crc & 1 == 1 { 0xEDB8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
        }
    }
    !crc
}
