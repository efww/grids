//! Windows outside this module may not paint their own chrome.
//!
//! Brushes, fonts, and GDI colour setters live here. A path in [`ALLOW`] is a window
//! that still draws itself; drop it from the list once that window calls the shared
//! painters. System dialogs (message boxes, the file picker, the colour picker) are
//! not listed: they do not call these.

use std::path::{Path, PathBuf};

const BANNED: &[&str] = &[
    "CreateSolidBrush",
    "CreateFont",
    "RGB(",
    "SetTextColor",
    "SetBkColor",
    "SetDCBrushColor",
    "SetDCPenColor",
];

/// Follow-up windows. Each one still contains a banned call. A new file that
/// paints on its own fails the test, and so does leaving a finished file here.
const ALLOW: &[&str] = &[
    "src/pane/pview.rs",
    "src/pane/select.rs",
    "src/pane/view.rs",
];

/// This pull request. These paint through the shared module, allow-list or not.
const MIGRATED: &[(&str, &[&str])] = &[
    (
        "src/excel/ui.rs",
        &[
            "paint_window(",
            "paint_check(",
            "paint_label(",
            "paint_button(",
            "style_dialog_caption(",
            "body_font(",
        ],
    ),
    (
        "src/ui/srvdlg.rs",
        &[
            "paint_window(",
            "paint_field(",
            "paint_segment(",
            "paint_check(",
            "paint_button(",
            "color_field_edit(",
            "style_popup_chrome(",
        ],
    ),
    (
        "src/basket.rs",
        &[
            "paint_window(",
            "paint_card(",
            "paint_button(",
            "paint_meta(",
            "paint_label(",
            "style_dialog_caption(",
            "body_font(",
            "sel_mist(",
        ],
    ),
    (
        "src/rename/win.rs",
        &[
            "paint_window(",
            "paint_card(",
            "paint_plate(",
            "paint_field(",
            "paint_button(",
            "paint_check(",
            "paint_label(",
            "paint_meta(",
            "paint_section_header(",
            "style_dialog_caption(",
            "body_font(",
            "color_field_edit(",
        ],
    ),
    (
        "src/tablist.rs",
        &[
            "paint_window(",
            "paint_card(",
            "paint_field(",
            "paint_chip(",
            "paint_wash(",
            "paint_button(",
            "paint_label(",
            "paint_meta(",
            "style_dialog_caption(",
            "body_font(",
            "color_field_edit(",
        ],
    ),
    (
        "src/settings/win.rs",
        &[
            "paint_window(",
            "paint_card(",
            "paint_field(",
            "paint_field_on(",
            "paint_chip(",
            "paint_wash(",
            "paint_button(",
            "paint_check(",
            "paint_segment(",
            "paint_label(",
            "paint_meta(",
            "paint_section_header(",
            "style_dialog_caption(",
            "body_font(",
            "color_field_edit(",
            "sel_mist(",
        ],
    ),
    (
        "src/watch/win.rs",
        &[
            "paint_window(",
            "paint_card(",
            "paint_button(",
            "paint_check(",
            "paint_label(",
            "paint_meta(",
            "style_dialog_caption(",
            "body_font(",
        ],
    ),
    (
        "src/session/pick.rs",
        &[
            "paint_window(",
            "paint_card(",
            "paint_field(",
            "paint_chip(",
            "paint_wash(",
            "paint_button(",
            "paint_label(",
            "paint_meta(",
            "style_dialog_caption(",
            "body_font(",
            "color_field_edit(",
            "sel_mist(",
        ],
    ),
    (
        "src/session/load.rs",
        &[
            "paint_window(",
            "paint_card(",
            "paint_card_fill(",
            "paint_field(",
            "paint_chip(",
            "paint_button(",
            "paint_label(",
            "paint_meta(",
            "style_dialog_caption(",
            "body_font(",
            "icon_font(",
            "color_field_edit(",
            "sel_mist(",
        ],
    ),
    (
        "src/watch/notify.rs",
        &[
            "paint_window(",
            "paint_card(",
            "paint_button(",
            "paint_label(",
            "paint_meta(",
            "style_dialog_caption(",
            "body_font(",
        ],
    ),
    (
        "src/shell/fastcopy.rs",
        &[
            "paint_window(",
            "paint_meter(",
            "paint_button(",
            "paint_check(",
            "paint_label(",
            "paint_meta(",
            "wrap_text(",
            "style_dialog_caption(",
            "style_popup_chrome(",
            "body_font(",
            "bold_ui_font(",
            "meta_font(",
        ],
    ),
    (
        "src/app/wnd.rs",
        &[
            "paint_window(",
            "paint_meta(",
            "draw_button(",
            "color_edit_on(",
            "paint_pane(",
            "style_dialog_caption(",
        ],
    ),
    (
        "src/pane/nav.rs",
        &[
            "paint_window(",
            "paint_button(",
            "paint_center_wrap(",
            "visual_from_pointer(",
        ],
    ),
    (
        "src/pane/peek.rs",
        &[
            "paint_window(",
            "paint_card(",
            "paint_button(",
            "paint_wash(",
            "paint_label(",
            "paint_meta(",
            "paint_center_meta(",
            "wrap_text(",
            "wrap_text_ellipsis(",
            "style_dialog_caption(",
            "style_preview_visuals(",
            "body_font(",
            "bold_ui_font(",
            "icon_font(",
            "visual_from_pointer(",
        ],
    ),
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn code_without_line_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let code = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        out.push_str(code);
        out.push('\n');
    }
    out
}

fn banned_hits(code: &str) -> Vec<&'static str> {
    BANNED
        .iter()
        .copied()
        .filter(|pat| code.contains(pat))
        .collect()
}

fn read_code(rel: &str) -> String {
    let path = crate_root().join(rel);
    let src =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    code_without_line_comments(&src)
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn rel_from_root(path: &Path) -> String {
    path.strip_prefix(crate_root())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_lines_do_not_count_as_paint_calls() {
        let code = code_without_line_comments(
            "// CreateSolidBrush\nlet _ = CreateFontIndirectW(&lf); // RGB(1, 2, 3)\n",
        );
        assert_eq!(banned_hits(&code), vec!["CreateFont"]);
        assert!(banned_hits("SetTextColor(dc, c)\nSetBkColor(dc, c)\n").len() >= 2);
        assert!(banned_hits("paint_button(dc, r)").is_empty());
    }

    #[test]
    fn windows_outside_ui_do_not_grow_their_own_brushes_fonts_or_colours() {
        let mut sorted = ALLOW.to_vec();
        sorted.sort_unstable();
        assert_eq!(ALLOW, sorted.as_slice(), "keep ALLOW sorted");

        let mut files = Vec::new();
        rust_files(&crate_root().join("src"), &mut files);
        let mut offenders = Vec::new();
        for path in files {
            let rel = rel_from_root(&path);
            if rel.starts_with("src/ui/") {
                continue;
            }
            let code = code_without_line_comments(&std::fs::read_to_string(&path).unwrap());
            let hits = banned_hits(&code);
            if hits.is_empty() {
                assert!(
                    !ALLOW.contains(&rel.as_str()),
                    "{rel} is on ALLOW but no longer paints on its own"
                );
                continue;
            }
            if !ALLOW.contains(&rel.as_str()) {
                offenders.push(format!("{rel}: {}", hits.join(", ")));
            }
        }
        assert!(
            offenders.is_empty(),
            "direct paint outside src/ui:\n{}",
            offenders.join("\n")
        );

        for rel in ALLOW {
            let hits = banned_hits(&read_code(rel));
            assert!(
                !hits.is_empty(),
                "{rel} is on ALLOW but no longer paints on its own: remove it"
            );
        }
    }

    #[test]
    fn migrated_windows_paint_only_through_the_shared_module() {
        for (rel, needles) in MIGRATED {
            assert!(
                !ALLOW.contains(rel),
                "{rel} is migrated and must not be exempt"
            );
            let code = read_code(rel);
            let hits = banned_hits(&code);
            assert!(hits.is_empty(), "{rel} paints on its own: {hits:?}");
            assert!(
                !code.contains("aa_rfill("),
                "{rel} still paints its own rounded rect"
            );
            for needle in *needles {
                assert!(code.contains(needle), "{rel} no longer calls {needle}");
            }
        }
    }
}
