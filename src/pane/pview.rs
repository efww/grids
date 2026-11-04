//! Shared scroll state and body painting for Quick Look and the preview pane.
//!
//! Text, tables and lists measure once and then draw only the rows on screen.
//! Each surface keeps its own [`BodyScroll`]; the two previews can be open together.

use crate::app::*;
use crate::i18n::*;
use crate::imports::*;
use crate::ui::*;

use super::peek::Pix;
use super::preview::TextKind;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Style {
    Plain,
    Mono, // not wrapped
    H1,
    H2,
    H3,
    Bullet,
    Quote,
}

pub(crate) struct Line {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) y: i32,
    pub(crate) h: i32,
    pub(crate) no: usize, // source line number, for the code gutter
    table: Option<(Vec<Vec<String>>, Vec<i32>)>,
}

/// Scroll, laid-out lines and the heading faces for one preview surface.
#[derive(Default)]
pub(crate) struct BodyScroll {
    pub y: i32,
    pub x: i32,
    pub max_y: i32,
    pub max_x: i32,
    pub page_h: i32,
    pub page_w: i32,
    pub layout: Option<(usize, i32, Vec<Line>, i32)>,
}

/// Heading faces cached by the caller. Quick Look keeps them on its view; the preview pane
/// keeps them in a thread-local so the shared scroll state can cross threads.
pub(crate) type FaceCache = Option<(i32, [HFONT; 4])>;

/// Markdown lines with their look: headings, bullets, quotes, fenced code. Inline marks are dropped.
pub(crate) fn md_lines(text: &str) -> Vec<(String, Style)> {
    let mut out = Vec::new();
    let mut code = false;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            code = !code;
            continue;
        }
        if code {
            out.push((l.to_string(), Style::Mono));
            continue;
        }
        let clean = |s: &str| s.replace("**", "").replace("__", "").replace('`', "");
        let h3 = t.strip_prefix("### ").or_else(|| t.strip_prefix("#### ")).or_else(|| t.strip_prefix("##### "));
        let item = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ "));
        let (text, style) = if let Some(h) = h3 {
            (clean(h), Style::H3)
        } else if let Some(h) = t.strip_prefix("## ") {
            (clean(h), Style::H2)
        } else if let Some(h) = t.strip_prefix("# ") {
            (clean(h), Style::H1)
        } else if let Some(b) = item {
            let indent = "    ".repeat((l.len() - t.len()) / 2);
            (format!("{indent}• {}", clean(b)), Style::Bullet)
        } else if let Some(q) = t.strip_prefix("> ").or_else(|| t.strip_prefix('>')) {
            (clean(q), Style::Quote)
        } else if t.len() >= 3 && t.chars().all(|c| c == '-') {
            ("────────".into(), Style::Quote)
        } else {
            (clean(l), Style::Plain)
        };
        out.push((text, style));
    }
    out
}

/// Source text, or the laid-out lines inside `sel` (1-based, inclusive) when a range is set.
pub(crate) fn selected_text(source: &str, lines: &[Line], sel: Option<(usize, usize)>) -> String {
    let Some((a, b)) = sel else { return source.to_string() };
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    if lo == 0 || hi == 0 {
        return source.to_string();
    }
    let picked: Vec<&str> = lines.iter().filter(|l| l.no >= lo && l.no <= hi).map(|l| l.text.as_str()).collect();
    if picked.is_empty() {
        source.to_string()
    } else {
        picked.join("\n")
    }
}

/// Row under the pointer. `scroll_y` is the list's vertical scroll. `None` past the last row.
pub(crate) fn list_row_at(scroll_y: i32, area_top: i32, y: i32, row_h: i32, len: usize) -> Option<usize> {
    if row_h <= 0 || y < area_top || len == 0 {
        return None;
    }
    let i = scroll_y.saturating_add(y - area_top) / row_h;
    if i < 0 {
        return None;
    }
    let i = i as usize;
    (i < len).then_some(i)
}

/// Source line number under the pointer, in the same space [`paint_text`] lays lines out.
pub(crate) fn line_no_at(lines: &[Line], scroll_y: i32, area_top: i32, y: i32) -> Option<usize> {
    if y < area_top {
        return None;
    }
    let rel = scroll_y.saturating_add(y - area_top);
    if rel < 0 {
        return None;
    }
    lines.iter().find(|l| rel >= l.y && rel < l.y.saturating_add(l.h)).map(|l| l.no)
}

pub(crate) fn row_h(fh: i32, dpi: i32) -> i32 {
    fh + 8 * dpi / 96
}

pub(crate) unsafe fn fonts(cache: &mut FaceCache, dpi: i32, base: HFONT) -> [HFONT; 4] {
    if let Some((d, f)) = *cache {
        if d == dpi {
            return f;
        }
        for x in f {
            delete_font(x);
        }
    }
    let mut lf = LOGFONTW::default();
    GetObjectW(base.into(), size_of::<LOGFONTW>() as i32, Some(&mut lf as *mut _ as _));
    let mono = CreateFontIndirectW(&LOGFONTW { lfFaceName: logfont("Consolas", 0).lfFaceName, ..lf });
    let bold = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, ..lf });
    let h1 = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, lfHeight: lf.lfHeight * 16 / 10, ..lf });
    let h2 = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, lfHeight: lf.lfHeight * 13 / 10, ..lf });
    let f = [mono, bold, h1, h2];
    *cache = Some((dpi, f));
    f
}

pub(crate) unsafe fn clear_faces(cache: &mut FaceCache) {
    if let Some((_, fonts)) = cache.take() {
        for f in fonts {
            delete_font(f);
        }
    }
}

fn style_font(st: Style, font: HFONT, f: &[HFONT; 4]) -> HFONT {
    match st {
        Style::Mono => f[0],
        Style::H1 => f[2],
        Style::H2 => f[3],
        Style::H3 => f[1],
        _ => font,
    }
}

// Heights of every line at this width, measured once per item and width; painting then draws only what shows.
unsafe fn layout(scroll: &mut BodyScroll, dc: HDC, gen: usize, width: i32, text: &str, kind: TextKind, font: HFONT, f: &[HFONT; 4], dpi: i32) -> i32 {
    if let Some(total) = scroll.layout.as_ref().filter(|(g, w, ..)| *g == gen && *w == width).map(|l| l.3) {
        return total;
    }
    let src: Vec<(String, Style)> = match kind {
        TextKind::Markdown => md_lines(text),
        TextKind::Code => text.lines().map(|l| (l.to_string(), Style::Mono)).collect(),
        TextKind::Plain => text.lines().map(|l| (l.to_string(), Style::Plain)).collect(),
        TextKind::Document => text.lines().map(|l| (l.to_string(), Style::Plain)).collect(),
    };
    let mut y = 0;
    let mut lines = Vec::with_capacity(src.len().min(40_000));
    let mut source = src.into_iter().take(40_000).enumerate().peekable();
    while let Some((no, (t, st))) = source.next() {
        if kind == TextKind::Document && t.starts_with("| ") {
            let mut rows = Vec::new();
            let parse = |line: &str| -> Vec<String> {
                line.strip_prefix("| ").unwrap_or(line).strip_suffix('|').unwrap_or(line).split(" | ").map(|cell| cell.trim().to_string()).collect()
            };
            rows.push(parse(&t));
            while source.peek().is_some_and(|(_, (line, _))| line.starts_with("| ")) {
                if let Some((_, (line, _))) = source.next() {
                    rows.push(parse(&line));
                }
            }
            let mut widths = Vec::<i32>::new();
            let scale = |v: i32| v * dpi / 96;
            for row in &rows {
                for (col, cell) in row.iter().enumerate() {
                    if widths.len() <= col {
                        widths.push(scale(36));
                    }
                    widths[col] = widths[col].max((text_w(dc, cell, font) + scale(14)).min(scale(260)));
                }
            }
            let rh = app(|a| a.fh) + scale(8);
            let h = rh * rows.len() as i32;
            lines.push(Line { text: String::new(), style: st, y, h, no: no + 1, table: Some((rows, widths)) });
            y += h;
            continue;
        }
        let old = SelectObject(dc, style_font(st, font, f).into());
        let mut tm = TEXTMETRICW::default();
        _ = GetTextMetricsW(dc, &mut tm);
        let lh = tm.tmHeight + tm.tmExternalLeading;
        let h = if st == Style::Mono || t.is_empty() {
            lh
        } else {
            let mut w: Vec<u16> = t.encode_utf16().collect();
            let mut r = RECT { left: 0, top: 0, right: width, bottom: 0 };
            DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_WORDBREAK | DT_EXPANDTABS | DT_NOPREFIX | DT_EDITCONTROL);
            (r.bottom - r.top).max(lh)
        };
        SelectObject(dc, old);
        y += match st {
            Style::H1 | Style::H2 => lh / 2,
            Style::H3 => lh / 3,
            _ => 0,
        };
        lines.push(Line { text: t, style: st, y, h, no: no + 1, table: None });
        y += h + if matches!(st, Style::H1 | Style::H2) { dpi / 24 } else { 0 };
    }
    scroll.layout = Some((gen, width, lines, y));
    y
}

pub(crate) fn clamp_scroll(scroll: &mut BodyScroll, total: i32, page: i32) {
    let page = page.max(0);
    scroll.page_h = page;
    scroll.max_y = total.saturating_sub(page).max(0);
    scroll.y = scroll.y.clamp(0, scroll.max_y.max(0));
    // Text, tables and lists keep their own sideways scroll and do not grow this bar.
    (scroll.max_x, scroll.page_w) = (0, 0);
}

// DrawTextW without text()'s single-line, end-ellipsis defaults.
pub(crate) unsafe fn text_flags(dc: HDC, mut r: RECT, s: &str, font: HFONT, rgb: u32, flags: DRAW_TEXT_FORMAT) {
    let vcenter = flags.0 & DT_VCENTER.0 != 0;
    let wrap = flags.0 & DT_WORDBREAK.0 != 0;
    let calc = flags.0 & DT_CALCRECT.0 != 0;
    if vcenter && !wrap && !calc {
        draw_ink_line(dc, r, s, font, rgb, flags);
        return;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return;
    }
    let old = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, cref(rgb));
    DrawTextW(dc, &mut w, &mut r, flags | DT_NOPREFIX);
    SelectObject(dc, old);
}

pub(crate) unsafe fn text_w(dc: HDC, s: &str, font: HFONT) -> i32 {
    let w: Vec<u16> = s.chars().take(200).collect::<String>().encode_utf16().collect();
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    _ = GetTextExtentPoint32W(dc, &w, &mut sz);
    SelectObject(dc, old);
    sz.cx
}

unsafe fn clip(dc: HDC, r: RECT, draw: impl FnOnce()) {
    let rgn = CreateRectRgn(r.left, r.top, r.right, r.bottom);
    SelectClipRgn(dc, Some(rgn));
    draw();
    SelectClipRgn(dc, None);
    _ = DeleteObject(rgn.into());
}

/// `sel` highlights source lines (1-based, inclusive). `accent` `0` draws no highlight.
pub(crate) unsafe fn paint_text(
    scroll: &mut BodyScroll,
    dc: HDC,
    area: RECT,
    gen: usize,
    text: &str,
    kind: TextKind,
    cut: bool,
    font: HFONT,
    dpi: i32,
    pic: Option<&Pix>,
    sel: Option<(usize, usize)>,
    accent: u32,
    faces: &mut FaceCache,
) {
    let s = |v: i32| v * dpi / 96;
    let f = fonts(faces, dpi, font);
    let gutter = if kind == TextKind::Code { s(44) } else { 0 };
    let width = area.right - area.left - gutter - s(8);
    let pic_size = pic.map(|p| {
        let k = ((area.right - area.left) as f32 / p.w as f32).min((area.bottom - area.top) as f32 / p.h as f32);
        ((p.w as f32 * k) as i32, (p.h as f32 * k) as i32)
    });
    let offset = pic_size.map_or(0, |(_, h)| h + s(20));
    let total = offset + layout(scroll, dc, gen, width, text, kind, font, &f, dpi) + if cut { s(28) } else { 0 };
    clamp_scroll(scroll, total, area.bottom - area.top);
    if kind == TextKind::Document {
        let full = scroll
            .layout
            .as_ref()
            .map(|(_, _, lines, _)| lines.iter().filter_map(|l| l.table.as_ref()).map(|(_, widths)| widths.iter().sum::<i32>()).max().unwrap_or(0))
            .unwrap_or(0);
        scroll.x = scroll.x.clamp(0, (full - (area.right - area.left)).max(0));
    }
    let (sy, sx) = (scroll.y, scroll.x);
    let mark = sel.map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
    clip(dc, area, || {
        if let (Some(p), Some((w, h))) = (pic, pic_size) {
            let x = (area.left + area.right - w) / 2;
            let y = area.top - sy;
            blit_bgra(dc, RECT { left: x, top: y, right: x + w, bottom: y + h }, p.w, p.h, &p.bgra);
        }
        let sy = sy - offset;
        let Some((_, _, lines, _)) = scroll.layout.as_ref() else { return };
        let first = lines.partition_point(|l| l.y + l.h < sy);
        let fh = app(|a| a.fh);
        for l in lines[first..].iter().take_while(|l| l.y - sy < area.bottom - area.top) {
            let top = area.top + l.y - sy;
            if let Some((rows, widths)) = &l.table {
                let rh = fh + s(8);
                let first = ((area.top - top).max(0) / rh) as usize;
                let shown = ((area.bottom - area.top) / rh + 2) as usize;
                for (row_no, row) in rows.iter().enumerate().skip(first).take(shown) {
                    let row_top = top + row_no as i32 * rh;
                    let mut x = area.left - sx;
                    for (col, w) in widths.iter().enumerate() {
                        if x + w > area.left && x < area.right {
                            let cell = row.get(col).map(String::as_str).unwrap_or("");
                            let rect = RECT { left: x + s(7), top: row_top, right: x + w - s(7), bottom: row_top + rh };
                            crate::text(dc, rect, cell, font, pal().text, DT_LEFT);
                            fill(dc, RECT { left: x + w - 1, top: row_top, right: x + w, bottom: row_top + rh }, pal().line);
                            fill(dc, RECT { left: x, top: row_top + rh - 1, right: x + w, bottom: row_top + rh }, pal().line);
                        }
                        x += w;
                    }
                    fill(dc, RECT { left: area.left - sx, top: row_top, right: area.left - sx + 1, bottom: row_top + rh }, pal().line);
                }
                let table_width: i32 = widths.iter().sum();
                fill(dc, RECT { left: area.left - sx, top, right: area.left - sx + table_width, bottom: top + 1 }, pal().line);
                continue;
            }
            if mark.is_some_and(|(lo, hi)| accent != 0 && l.no >= lo && l.no <= hi) {
                fill(dc, RECT { left: area.left, top, right: area.right, bottom: top + l.h }, blend(pal().card, accent, 0.22));
            }
            if gutter > 0 {
                let g = RECT { left: area.left, top, right: area.left + gutter - s(10), bottom: top + l.h };
                text_flags(dc, g, &l.no.to_string(), f[0], pal().sub, DT_RIGHT | DT_SINGLELINE);
            }
            let mono = l.style == Style::Mono;
            let (ink, fnt) = match l.style {
                Style::Quote => (pal().sub, font),
                st => (pal().text, style_font(st, font, &f)),
            };
            let left = area.left + gutter - if mono { sx } else { 0 };
            let r = RECT { left, top, right: if mono { left + 100_000 } else { area.right - s(8) }, bottom: top + l.h };
            let flags = if mono { DT_SINGLELINE | DT_EXPANDTABS } else { DT_WORDBREAK | DT_EXPANDTABS | DT_EDITCONTROL };
            text_flags(dc, r, &l.text, fnt, ink, flags);
        }
        if cut {
            let y = area.top + total - offset - s(24) - sy;
            let r = RECT { left: area.left, top: y, right: area.right, bottom: y + s(24) };
            text_flags(dc, r, tr(crate::Key::OnlyFirst512KbIsShown), font, pal().sub, DT_SINGLELINE);
        }
    });
}

// A grid: column widths from the first rows' text, the first row as a header, scrolled both ways.
pub(crate) unsafe fn paint_table(scroll: &mut BodyScroll, dc: HDC, area: RECT, rows: &[Vec<String>], cut: bool, font: HFONT, dpi: i32, faces: &mut FaceCache) {
    let s = |v: i32| v * dpi / 96;
    let f = fonts(faces, dpi, font);
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 {
        text(dc, area, tr(crate::Key::Empty), font, pal().sub, DT_CENTER);
        return;
    }
    let mut widths = vec![s(36); cols];
    for (k, r) in rows.iter().take(200).enumerate() {
        for (c, cell) in r.iter().enumerate() {
            let w = text_w(dc, cell, if k == 0 { f[1] } else { font }) + s(14);
            widths[c] = widths[c].max(w.min(s(260)));
        }
    }
    let num = text_w(dc, &rows.len().to_string(), font) + s(14);
    let rh = app(|a| a.fh) + s(8);
    let total = rh * rows.len() as i32 + if cut { rh } else { 0 };
    clamp_scroll(scroll, total, area.bottom - area.top);
    let full: i32 = num + widths.iter().sum::<i32>();
    scroll.x = scroll.x.min((full - (area.right - area.left)).max(0)).max(0);
    let (sy, sx) = (scroll.y, scroll.x);
    clip(dc, area, || {
        let first = (sy / rh) as usize;
        let shown = ((area.bottom - area.top) / rh + 2) as usize;
        for (k, r) in rows.iter().enumerate().skip(first).take(shown) {
            let top = area.top + k as i32 * rh - sy;
            let row = RECT { left: area.left, top, right: area.right, bottom: top + rh };
            if k == 0 {
                fill(dc, row, pal().pill);
            }
            fill(dc, RECT { top: row.bottom - 1, ..row }, pal().line);
            text(dc, RECT { left: area.left, right: area.left + num - s(6), ..row }, &(k + 1).to_string(), font, pal().sub, DT_RIGHT);
            let mut x = area.left + num - sx;
            for (c, w) in widths.iter().enumerate() {
                if x + w > area.left + num && x < area.right {
                    let cell = r.get(c).map(String::as_str).unwrap_or("");
                    let cr = RECT { left: (x + s(7)).max(area.left + num), top, right: x + w - s(7), bottom: top + rh };
                    let right = k > 0 && !cell.is_empty() && cell.replace(',', "").parse::<f64>().is_ok();
                    text(dc, cr, cell, if k == 0 { f[1] } else { font }, pal().text, if right { DT_RIGHT } else { DT_LEFT });
                    fill(dc, RECT { left: x + w - 1, top, right: x + w, bottom: top + rh }, pal().line);
                }
                x += w;
            }
        }
        if cut {
            let top = area.top + rows.len() as i32 * rh - sy;
            let r = RECT { left: area.left + s(6), top, right: area.right, bottom: top + rh };
            text(dc, r, tr(crate::Key::MoreRowsNotShown), font, pal().sub, DT_LEFT);
        }
    });
}

pub(crate) unsafe fn paint_list(scroll: &mut BodyScroll, dc: HDC, area: RECT, rows: &[(String, String)], font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    let rh = app(|a| a.fh) + s(8);
    let top0 = area.top;
    if rows.is_empty() {
        text(dc, area, tr(crate::Key::Empty), font, pal().sub, DT_CENTER);
        return;
    }
    let page = area.bottom - top0;
    clamp_scroll(scroll, rh * rows.len() as i32, page);
    let sy = scroll.y;
    let sw = s(90);
    clip(dc, RECT { top: top0, ..area }, || {
        for (k, (name, size)) in rows.iter().enumerate().skip((sy / rh) as usize).take((page / rh + 2) as usize) {
            let top = top0 + k as i32 * rh - sy;
            let r = RECT { left: area.left, top, right: area.right, bottom: top + rh };
            if k % 2 == 1 {
                fill(dc, RECT { left: r.left - s(8), right: r.right + s(8), ..r }, blend(pal().card, pal().bg, 0.45));
            }
            text(dc, RECT { right: r.right - sw - s(8), ..r }, name, font, pal().text, DT_LEFT);
            text(dc, RECT { left: r.right - sw, ..r }, size, font, pal().sub, DT_RIGHT);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_or_whole_text() {
        let lines = vec![
            Line { text: "one".into(), style: Style::Plain, y: 0, h: 10, no: 1, table: None },
            Line { text: "two".into(), style: Style::Plain, y: 10, h: 10, no: 2, table: None },
            Line { text: "three".into(), style: Style::Plain, y: 20, h: 10, no: 3, table: None },
        ];
        assert_eq!(selected_text("one\ntwo\nthree", &lines, None), "one\ntwo\nthree");
        assert_eq!(selected_text("one\ntwo\nthree", &lines, Some((2, 3))), "two\nthree");
        assert_eq!(selected_text("one\ntwo\nthree", &lines, Some((3, 1))), "one\ntwo\nthree");
        assert_eq!(selected_text("all", &lines, Some((9, 9))), "all");
    }

    #[test]
    fn list_and_line_hits() {
        assert_eq!(list_row_at(0, 100, 100, 20, 3), Some(0));
        assert_eq!(list_row_at(20, 100, 110, 20, 3), Some(1));
        assert_eq!(list_row_at(0, 100, 99, 20, 3), None);
        assert_eq!(list_row_at(0, 100, 160, 20, 3), None);
        let lines = vec![Line { text: "a".into(), style: Style::Mono, y: 0, h: 16, no: 1, table: None }];
        assert_eq!(line_no_at(&lines, 0, 40, 48), Some(1));
        assert_eq!(line_no_at(&lines, 0, 40, 39), None);
    }
}
