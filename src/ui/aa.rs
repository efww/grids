//! Supersampled shapes, the same path the settings toggles use.
//! Draw at [`AA_SCALE`] into a DIB cleared to the parent colour, box-filter, and copy down.
//! No GDI+, no Direct2D.

use crate::imports::*;

/// Supersample factor. 3× is enough for a knob or a corner.
pub(crate) const AA_SCALE: i32 = 3;
/// Parent-coloured pixels around the shape so a curve that crosses a sample block can blend.
pub(crate) const AA_PAD: i32 = 2;

pub(crate) fn scaled_from(outer: RECT, part: RECT, scale: i32, ox: i32, oy: i32) -> RECT {
    RECT {
        left: ox + (part.left - outer.left) * scale,
        top: oy + (part.top - outer.top) * scale,
        right: ox + (part.right - outer.left) * scale,
        bottom: oy + (part.bottom - outer.top) * scale,
    }
}

/// Average each `scale`×`scale` block. Flat fills stay exact; only an edge that mixes both colours softens.
pub(crate) fn box_down(src: &[u8], sw: usize, dst: &mut [u8], dw: usize, dh: usize, scale: usize) {
    let area = (scale * scale) as u32;
    for y in 0..dh {
        for x in 0..dw {
            let mut b = 0u32;
            let mut g = 0u32;
            let mut rch = 0u32;
            let y0 = y * scale;
            let x0 = x * scale;
            for sy in 0..scale {
                let row = (y0 + sy) * sw + x0;
                for sx in 0..scale {
                    let i = (row + sx) * 4;
                    b += src[i] as u32;
                    g += src[i + 1] as u32;
                    rch += src[i + 2] as u32;
                }
            }
            let o = (y * dw + x) * 4;
            dst[o] = (b / area) as u8;
            dst[o + 1] = (g / area) as u8;
            dst[o + 2] = (rch / area) as u8;
            // Already composited against `ground`. Opaque so the buffered DC does not blend it again.
            dst[o + 3] = 255;
        }
    }
}

/// Average each `scale`×`scale` block of a one-byte coverage mask.
/// The star no longer uses this. The test keeps the filter honest.
#[cfg(test)]
pub(crate) fn box_down_cov(src: &[u8], sw: usize, dst: &mut [u8], dw: usize, dh: usize, scale: usize) {
    let area = (scale * scale) as u32;
    if area == 0 {
        return;
    }
    for y in 0..dh {
        for x in 0..dw {
            let mut s = 0u32;
            let y0 = y * scale;
            let x0 = x * scale;
            for sy in 0..scale {
                let row = (y0 + sy) * sw + x0;
                for sx in 0..scale {
                    s += src[row + sx] as u32;
                }
            }
            dst[y * dw + x] = (s / area) as u8;
        }
    }
}

unsafe fn dib_section(dc: HDC, w: i32, h: i32) -> Option<(HBITMAP, *mut u8)> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    if bits.is_null() {
        let _ = DeleteObject(bmp.into());
        return None;
    }
    Some((bmp, bits.cast()))
}

/// Draw the shape at `AA_SCALE` into a DIB cleared to `ground`, box-filter it, and copy it down.
/// `paint` gets the memory DC, the scale, and the origin of `r` inside the padded bitmap.
pub(crate) unsafe fn aa_blit(dc: HDC, r: RECT, ground: u32, paint: impl FnOnce(HDC, i32, i32, i32)) -> bool {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || h <= 0 || w > 2400 || h > 800 {
        return false;
    }
    let scale = AA_SCALE;
    let pad = AA_PAD;
    let (bw, bh) = (w + pad * 2, h + pad * 2);
    let (sw, sh) = (bw * scale, bh * scale);
    let Some((big, big_bits)) = dib_section(dc, sw, sh) else { return false };
    let mem = CreateCompatibleDC(Some(dc));
    if mem.is_invalid() {
        let _ = DeleteObject(big.into());
        return false;
    }
    let old = SelectObject(mem, big.into());
    crate::fill(mem, RECT { left: 0, top: 0, right: sw, bottom: sh }, ground);
    paint(mem, scale, pad * scale, pad * scale);
    let _ = GdiFlush();
    SelectObject(mem, old);
    let _ = DeleteDC(mem);

    let Some((small, small_bits)) = dib_section(dc, bw, bh) else {
        let _ = DeleteObject(big.into());
        return false;
    };
    box_down(
        std::slice::from_raw_parts(big_bits, (sw as usize) * (sh as usize) * 4),
        sw as usize,
        std::slice::from_raw_parts_mut(small_bits, (bw as usize) * (bh as usize) * 4),
        bw as usize,
        bh as usize,
        scale as usize,
    );
    let _ = DeleteObject(big.into());

    let mem = CreateCompatibleDC(Some(dc));
    if mem.is_invalid() {
        let _ = DeleteObject(small.into());
        return false;
    }
    let old = SelectObject(mem, small.into());
    let copied = StretchBlt(dc, r.left - pad, r.top - pad, bw, bh, Some(mem), 0, 0, bw, bh, SRCCOPY);
    SelectObject(mem, old);
    let _ = DeleteDC(mem);
    let _ = DeleteObject(small.into());
    copied.as_bool()
}

pub(crate) unsafe fn aa_rfill(dc: HDC, r: RECT, rad: i32, fill: u32, border: Option<u32>, ground: u32) {
    let painted = aa_blit(dc, r, ground, |mem, scale, ox, oy| unsafe {
        paint_round(mem, scaled_from(r, r, scale, ox, oy), rad.max(1) * scale, fill, border, scale);
    });
    if !painted {
        rfill(dc, r, rad, fill, border);
    }
}

/// Border is a 1px ring: the outer shape is the border colour, the fill sits `inset` source pixels in.
pub(crate) unsafe fn paint_round(dc: HDC, r: RECT, rad: i32, fill: u32, border: Option<u32>, inset: i32) {
    let Some(edge) = border.filter(|c| *c != fill) else {
        rfill(dc, r, rad.max(1), fill, None);
        return;
    };
    rfill(dc, r, rad.max(1), edge, None);
    let inner = RECT { left: r.left + inset, top: r.top + inset, right: r.right - inset, bottom: r.bottom - inset };
    if inner.right <= inner.left || inner.bottom <= inner.top {
        return;
    }
    rfill(dc, inner, (rad - inset).max(1), fill, None);
}

pub(crate) unsafe fn rfill(dc: HDC, r: RECT, rad: i32, fill: u32, border: Option<u32>) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let brush = CreateSolidBrush(crate::cref(fill));
    let pen = match border {
        Some(c) => CreatePen(PS_SOLID, 1, crate::cref(c)),
        None => HPEN(GetStockObject(NULL_PEN).0),
    };
    let op = SelectObject(dc, pen.into());
    let ob = SelectObject(dc, brush.into());
    let _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, rad.max(1), rad.max(1));
    SelectObject(dc, op);
    SelectObject(dc, ob);
    let _ = DeleteObject(brush.into());
    if border.is_some() {
        let _ = DeleteObject(pen.into());
    }
}

pub(crate) unsafe fn ellipse(dc: HDC, r: RECT, fill: u32, border: Option<u32>) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let brush = CreateSolidBrush(crate::cref(fill));
    let pen = match border {
        Some(c) => CreatePen(PS_SOLID, 1, crate::cref(c)),
        None => HPEN(GetStockObject(NULL_PEN).0),
    };
    let op = SelectObject(dc, pen.into());
    let ob = SelectObject(dc, brush.into());
    let _ = Ellipse(dc, r.left, r.top, r.right, r.bottom);
    SelectObject(dc, op);
    SelectObject(dc, ob);
    let _ = DeleteObject(brush.into());
    if border.is_some() {
        let _ = DeleteObject(pen.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_down_keeps_a_flat_fill_and_blends_an_edge() {
        let scale = 3usize;
        let (sw, sh) = (6usize, 3usize);
        let mut src = vec![0u8; sw * sh * 4];
        for y in 0..sh {
            for x in 0..sw {
                let i = (y * sw + x) * 4;
                if x < 3 {
                    src[i + 2] = 255;
                } else {
                    src[i] = 255;
                }
                src[i + 3] = 255;
            }
        }
        let mut dst = vec![0u8; 8];
        box_down(&src, sw, &mut dst, 2, 1, scale);
        assert_eq!(&dst[0..4], &[0, 0, 255, 255]);
        assert_eq!(&dst[4..8], &[255, 0, 0, 255]);
        for y in 0..sh {
            let i = (y * sw + 3) * 4;
            src[i] = 0;
            src[i + 2] = 255;
        }
        box_down(&src, sw, &mut dst, 2, 1, scale);
        assert_eq!(dst[4], (255u32 * 6 / 9) as u8);
        assert_eq!(dst[5], 0);
        assert_eq!(dst[6], (255u32 * 3 / 9) as u8);
        assert_eq!(dst[7], 255);
    }

    #[test]
    fn box_down_cov_averages_one_hot_sample_in_the_block() {
        let mut src = vec![0u8; 9];
        src[0] = 255;
        let mut dst = [0u8; 1];
        box_down_cov(&src, 3, &mut dst, 1, 1, 3);
        assert_eq!(dst[0], (255 / 9) as u8);
    }
}
