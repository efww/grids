// HWP/HWPX pages for the preview, drawn by rhwp (MIT, github.com/edwardkim/rhwp). The document is laid out
// once and written as a PDF under %TEMP%\grids-hwp, which the PDF preview then shows like any other PDF:
// first the opening page alone (quick), then the whole document while that page is already on screen.
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use crate::preview::PDF_MAX;

pub(crate) fn is_hwp(path: &str) -> bool {
    matches!(crate::preview::ext_of(path).as_str(), "hwp" | "hwpx" | "hwpt")
}

thread_local! {
    // the last laid-out document, so the whole-document PDF right after the first page skips a second layout
    static LAST: RefCell<Option<(u64, rhwp::DocumentCore)>> = const { RefCell::new(None) };
}

// The same file unchanged gives the same name; an edited one a new name.
fn key(path: &str) -> Option<u64> {
    let m = std::fs::metadata(path).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path, m.len(), m.modified().ok()).hash(&mut h);
    Some(h.finish())
}

fn dir() -> PathBuf {
    std::env::temp_dir().join("grids-hwp")
}

// Laid out documents are cheap to remake: anything older than a day goes when a new one is written.
fn sweep(d: &Path) {
    let old = std::time::Duration::from_secs(24 * 3600);
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        if e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a > old) {
            _ = std::fs::remove_file(e.path());
        }
    }
}

fn has_font(file: &str) -> bool {
    let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let user = std::env::var("LOCALAPPDATA").map(|l| Path::new(&l).join(r"Microsoft\Windows\Fonts").join(file));
    Path::new(&sys).join("Fonts").join(file).exists() || user.is_ok_and(|p| p.exists())
}

// When none of a run's fonts is installed rhwp falls back to these, and a fallback that is missing too leaves
// the text blank. Batang is not on every Windows; Malgun Gothic is.
fn options() -> rhwp::renderer::pdf::PdfExportOptions {
    let mut o = rhwp::renderer::pdf::PdfExportOptions::default();
    let sans = "맑은 고딕".to_string();
    if !has_font("batang.ttc") {
        o.fallback_serif = sans.clone();
    }
    o.fallback_mono = sans;
    o.embed_text = false; // outlines only: a third of the memory, and nobody selects text in a preview
    o
}

fn layout(path: &str) -> Option<rhwp::DocumentCore> {
    let bytes = std::fs::read(path).ok()?;
    let opened = rhwp::service::DocumentService::new()
        .with_title_scan(false)
        .open_bytes(&bytes, &rhwp::service::OpenOptions::new())
        .ok()?;
    Some(opened.into_core())
}

fn write(file: &Path, pdf: &[u8]) -> Option<()> {
    let d = file.parent()?;
    std::fs::create_dir_all(d).ok()?;
    sweep(d);
    // whole or not at all: another worker may open the same name at any moment
    let tmp = file.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, pdf).ok()?;
    std::fs::rename(&tmp, file).ok().or_else(|| {
        _ = std::fs::remove_file(&tmp);
        file.exists().then_some(())
    })
}

// A broken or unusual document must not take the window down with it: rhwp runs behind catch_unwind.
fn guarded<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
    catch_unwind(AssertUnwindSafe(f)).ok().flatten()
}

/// The first page as a PDF, and how many pages the preview offers (at most PDF_MAX).
pub(crate) fn first_pdf(path: &str) -> Option<(PathBuf, u32)> {
    let k = key(path)?;
    guarded(|| {
        let core = layout(path)?;
        let total = core.page_count().min(PDF_MAX);
        if total == 0 {
            return None;
        }
        let file = dir().join(format!("{k:016x}-1.pdf"));
        if !file.exists() {
            write(&file, &core.render_pages_pdf_native_with_options(&[0], &options()).ok()?)?;
        }
        LAST.with(|l| *l.borrow_mut() = Some((k, core)));
        Some((file, total))
    })
}

/// Every page the preview offers, as one PDF (made once per version of the file).
pub(crate) fn full_pdf(path: &str) -> Option<PathBuf> {
    let k = key(path)?;
    let file = dir().join(format!("{k:016x}.pdf"));
    if file.exists() {
        return Some(file);
    }
    guarded(|| {
        let core = match LAST.with(|l| l.borrow_mut().take()) {
            Some((lk, c)) if lk == k => c,
            _ => layout(path)?,
        };
        let pages: Vec<u32> = (0..core.page_count().min(PDF_MAX)).collect();
        write(&file, &core.render_pages_pdf_native_with_options(&pages, &options()).ok()?)?;
        Some(file)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::{pdf_open, pdf_page};

    // straight to stdout: the harness only captures print!/println!, and CI runs the tests without --nocapture
    macro_rules! say {
        ($($a:tt)*) => {{ use std::io::Write; _ = writeln!(std::io::stdout(), $($a)*); }};
    }

    // rhwp's own sample documents (with Hancom's PDFs of them in the same folder), fetched at test time
    const SAMPLES: &str = "https://raw.githubusercontent.com/edwardkim/rhwp/v0.9.0/samples/";

    fn fetch(name: &str, to: &Path) -> bool {
        let url = format!("{SAMPLES}{}", name.replace(' ', "%20"));
        std::process::Command::new("curl.exe").args(["-sfL", "-o"]).arg(to).arg(url).status().is_ok_and(|s| s.success())
    }

    // The page as coarse text art in the log: enough to see text lines, tables and blanks.
    fn art(p: &crate::peek::Pix) -> (f64, String) {
        let (cols, rows) = (60, 40);
        let (mut out, mut dark_all) = (String::new(), 0usize);
        for r in 0..rows {
            for c in 0..cols {
                let (x0, x1) = (c * p.w as usize / cols, (c + 1) * p.w as usize / cols);
                let (y0, y1) = (r * p.h as usize / rows, (r + 1) * p.h as usize / rows);
                let mut dark = 0;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = (y * p.w as usize + x) * 4;
                        let lum = (p.bgra[i] as u32 + p.bgra[i + 1] as u32 + p.bgra[i + 2] as u32) / 3;
                        dark += (lum < 160) as usize;
                    }
                }
                dark_all += dark;
                let f = dark as f64 / ((x1 - x0) * (y1 - y0)).max(1) as f64;
                out.push(if f > 0.25 { '#' } else if f > 0.08 { '+' } else if f > 0.01 { '.' } else { ' ' });
            }
            out.push('\n');
        }
        (dark_all as f64 * 100.0 / (p.w * p.h) as f64, out)
    }

    #[test]
    fn hwp_pages_on_windows() {
        unsafe { _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
        say!("HWPTEST fonts: batang={} malgun={}", has_font("batang.ttc"), has_font("malgun.ttf"));
        let d = std::env::temp_dir().join("grids-hwp-test");
        _ = std::fs::create_dir_all(&d);
        let docs = ["21868765_별표2_보건소_분장사무.hwp", "21_언어_기출_편집가능본.hwp", "2025년 기부·답례품 실적 지자체 보고서_양식.hwpx"];
        let mut drawn = 0;
        for (i, name) in docs.iter().enumerate() {
            let ext = name.rsplit('.').next().unwrap();
            let f = d.join(format!("s{i}.{ext}"));
            if !fetch(name, &f) {
                say!("HWPTEST {name}: download failed");
                continue;
            }
            let f = f.to_string_lossy().into_owned();
            let t = std::time::Instant::now();
            let first = first_pdf(&f);
            let t1 = t.elapsed().as_millis();
            let full = full_pdf(&f);
            let t2 = t.elapsed().as_millis();
            let pages = pdf_open(&f).ok().and_then(|doc| unsafe { pdf_page(&doc, 0, 600, 850) });
            say!("HWPTEST {name}: first={:?} first_ms={t1} full={} full_ms={t2}", first.as_ref().map(|x| x.1), full.is_some());
            if let Some(p) = pages {
                let (pct, a) = art(&p);
                say!("HWPTEST page 1 {}x{} dark={pct:.2}%\n{a}", p.w, p.h);
                drawn += (pct > 0.5) as usize;
            }
        }
        // a broken file and a cut-off real one: no page, and no panic out of rhwp
        let junk = d.join("junk.hwp");
        std::fs::write(&junk, (0..70000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect::<Vec<_>>()).unwrap();
        say!("HWPTEST junk: {:?}", first_pdf(&junk.to_string_lossy()).map(|x| x.1));
        let src = d.join("s0.hwp");
        if let Ok(b) = std::fs::read(&src) {
            let cut = d.join("cut.hwp");
            std::fs::write(&cut, &b[..b.len() / 2]).unwrap();
            say!("HWPTEST cut: {:?}", first_pdf(&cut.to_string_lossy()).map(|x| x.1));
        }
        assert_eq!(drawn, docs.len(), "every sample's first page has ink");
    }
}
