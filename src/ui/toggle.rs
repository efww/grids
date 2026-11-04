//! Settings toggle. Moved here so later windows can share it.
//! The capsule, knob, and colours are the ones Settings already painted.

use crate::imports::*;

/// Same scale the settings layout uses (`dip * dpi / 96`, no floor at 96).
fn sc(dpi: i32, v: i32) -> i32 {
    v * dpi / 96
}

pub(crate) fn toggle_knob_rect(r: RECT, on: bool, dpi: i32) -> RECT {
    let k = sc(dpi, 12);
    let pad = sc(dpi, 3);
    let x = if on { r.right - pad - k } else { r.left + pad };
    let y = r.top + (r.bottom - r.top - k) / 2;
    RECT { left: x, top: y, right: x + k, bottom: y + k }
}

pub(crate) unsafe fn draw_toggle(dc: HDC, r: RECT, on: bool, enabled: bool, accent: u32, dpi: i32) {
    // 40×20 pill. Off and disabled stay an outlined gray track on the white card.
    let (fill, border, knob) = if !enabled || !on {
        (crate::pal().card, crate::pal().sub, crate::pal().sub)
    } else {
        (accent, accent, crate::settings::on_chrome(accent))
    };
    let rad = (r.bottom - r.top).max(1);
    let knob_r = toggle_knob_rect(r, on, dpi);
    // One pass so the knob's edge blends into the pill, not into a second square blit.
    let painted = crate::aa_blit(dc, r, crate::pal().card, |mem, scale, ox, oy| unsafe {
        crate::paint_round(mem, crate::scaled_from(r, r, scale, ox, oy), rad * scale, fill, Some(border), scale);
        crate::ellipse(mem, crate::scaled_from(r, knob_r, scale, ox, oy), knob, None);
    });
    if !painted {
        crate::rfill(dc, r, rad, fill, Some(border));
        crate::ellipse(dc, knob_r, knob, None);
    }
}
