// HWP/HWPX pages for the preview, drawn by rhwp (MIT, github.com/edwardkim/rhwp). The document is laid out
// once per worker visit; each page is written as its own small PDF under %TEMP%\grids-hwp when the preview
// asks for it, and the PDF preview shows it like any other. One page at a time keeps the single preview worker
// free: moving to another file waits for at most one page, never for the whole document.
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

// How long each step took, in %TEMP%\grids-hwp\log.txt.
pub(crate) fn log(line: String) {
    use std::io::Write;
    let d = dir();
    _ = std::fs::create_dir_all(&d);
    let file = d.join("log.txt");
    if std::fs::metadata(&file).is_ok_and(|m| m.len() > 256 * 1024) {
        _ = std::fs::remove_file(&file); // a few days of timings is plenty
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
        _ = writeln!(f, "{line}");
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

// Some installed fonts (Hancom's 휴먼명조 among them) answer to their name but give no glyph outlines: rhwp
// picks them first, the text comes out blank, and the page is slow. Such a family is taken out of the page's
// font lists, so the next name in the list (Batang, Malgun Gothic, ...) draws the text instead.
struct Sink;
impl ttf_parser::OutlineBuilder for Sink {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

// A face is usable when it parses and the sample characters it maps have outlines.
fn usable(db: &fontdb::Database, id: fontdb::ID) -> bool {
    db.with_face_data(id, |data, i| {
        let Ok(f) = ttf_parser::Face::parse(data, i) else { return false };
        let mapped: Vec<_> = ['가', 'A', '1'].iter().filter_map(|&c| f.glyph_index(c)).collect();
        mapped.is_empty() || mapped.iter().any(|&g| f.outline_glyph(g, &mut Sink).is_some())
    })
    .unwrap_or(false)
}

// Installed under this name, but some face of it cannot be drawn.
fn broken_family(db: &fontdb::Database, name: &str) -> bool {
    db.faces()
        .filter(|f| f.families.iter().any(|(n, _)| n == name || n.eq_ignore_ascii_case(name)))
        .any(|f| !usable(db, f.id))
}

fn system_fonts() -> &'static fontdb::Database {
    static DB: std::sync::OnceLock<fontdb::Database> = std::sync::OnceLock::new();
    DB.get_or_init(|| {
        let t = std::time::Instant::now();
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        log(format!("fonts {} faces {}ms", db.len(), t.elapsed().as_millis()));
        db
    })
}

// Every font-family="a,b,c" of the page with the broken names left out; the names dropped, for the log.
fn drop_broken_fonts(svg: &str, broken: &mut impl FnMut(&str) -> bool) -> (String, Vec<String>) {
    const ATTR: &str = "font-family=\"";
    let (mut out, mut rest, mut dropped) = (String::with_capacity(svg.len()), svg, Vec::new());
    let mut seen = std::collections::HashMap::<String, String>::new();
    while let Some(i) = rest.find(ATTR) {
        out.push_str(&rest[..i + ATTR.len()]);
        rest = &rest[i + ATTR.len()..];
        let Some(end) = rest.find('"') else { break };
        let list = &rest[..end];
        let kept = seen.entry(list.to_string()).or_insert_with(|| {
            let kept: Vec<&str> = list
                .split(',')
                .filter(|item| {
                    let name = item.trim().trim_matches(|c| c == '\'' || c == '"');
                    let name = name.trim_start_matches("&apos;").trim_end_matches("&apos;");
                    let name = name.trim_start_matches("&quot;").trim_end_matches("&quot;");
                    let bad = !name.is_empty() && broken(name);
                    if bad && !dropped.iter().any(|d| d == name) {
                        dropped.push(name.to_string());
                    }
                    !bad
                })
                .collect();
            if kept.is_empty() { "serif".to_string() } else { kept.join(",") }
        });
        out.push_str(kept);
        rest = &rest[end..];
    }
    out.push_str(rest);
    (out, dropped)
}

fn page_pdf_bytes(c: &rhwp::DocumentCore, i: u32) -> Option<Vec<u8>> {
    let svg = c.render_page_svg_layer_with_profile_native(i, rhwp::paint::RenderProfile::Print).ok()?;
    let db = system_fonts();
    static VERDICT: std::sync::Mutex<Option<std::collections::HashMap<String, bool>>> = std::sync::Mutex::new(None);
    let (svg, dropped) = drop_broken_fonts(&svg, &mut |name| {
        let mut v = VERDICT.lock().unwrap_or_else(|e| e.into_inner());
        *v.get_or_insert_with(Default::default).entry(name.to_string()).or_insert_with(|| broken_family(db, name))
    });
    if !dropped.is_empty() {
        log(format!("page {} fonts skipped (no outlines): {}", i + 1, dropped.join(", ")));
    }
    rhwp::renderer::pdf::svgs_to_pdf_with_options(&[svg], &options()).ok()
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

// The laid-out document, from the worker's last visit when it is the same file unchanged.
fn with_core<T>(path: &str, k: u64, f: impl FnOnce(&rhwp::DocumentCore) -> Option<T>) -> Option<T> {
    LAST.with(|l| {
        let mut l = l.borrow_mut();
        if l.as_ref().is_none_or(|(lk, _)| *lk != k) {
            *l = None; // let the old one go before laying out the next
            let t = std::time::Instant::now();
            let core = layout(path);
            log(format!("layout {}ms ok={} {path}", t.elapsed().as_millis(), core.is_some()));
            *l = Some((k, core?));
        }
        f(&l.as_ref()?.1)
    })
}

/// How many pages the preview offers (at most PDF_MAX); None when rhwp cannot read the file.
pub(crate) fn page_count(path: &str) -> Option<u32> {
    let k = key(path)?;
    guarded(|| with_core(path, k, |c| Some(c.page_count().min(PDF_MAX))).filter(|&n| n > 0))
}

/// Page i (0-based) as a one-page PDF, made once per version of the file.
pub(crate) fn page_pdf(path: &str, i: u32) -> Option<PathBuf> {
    let k = key(path)?;
    let file = dir().join(format!("{k:016x}-{}.pdf", i + 1));
    if file.exists() {
        return Some(file);
    }
    guarded(|| {
        let t = std::time::Instant::now();
        let pdf = with_core(path, k, |c| page_pdf_bytes(c, i))?;
        let r = t.elapsed().as_millis();
        write(&file, &pdf)?;
        log(format!("page {} render {r}ms write {}ms {}KB", i + 1, t.elapsed().as_millis() - r, pdf.len() / 1024));
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
    fn broken_font_names_leave_the_lists() {
        let svg = r#"<text font-family="&apos;휴먼명조&apos;,&apos;Batang&apos;,serif">가</text><text font-family="'휴먼명조'">나</text>"#;
        let (out, dropped) = drop_broken_fonts(svg, &mut |n| n == "휴먼명조");
        assert_eq!(out, r#"<text font-family="&apos;Batang&apos;,serif">가</text><text font-family="serif">나</text>"#);
        assert_eq!(dropped, ["휴먼명조"]);
        assert_eq!(drop_broken_fonts(svg, &mut |_| false).0, svg);
    }

    #[test]
    fn font_without_outlines_is_broken() {
        let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let Ok(good) = std::fs::read(Path::new(&sys).join(r"Fonts\malgun.ttf")) else { return };
        // the same font with its glyph outlines wiped
        let mut bad = good.clone();
        let glyf = ttf_parser::RawFace::parse(&good, 0).unwrap().table(ttf_parser::Tag::from_bytes(b"glyf")).unwrap();
        let at = glyf.as_ptr() as usize - good.as_ptr() as usize;
        bad[at..at + glyf.len()].fill(0);
        for (data, want) in [(good, true), (bad, false)] {
            let mut db = fontdb::Database::new();
            db.load_font_data(data);
            let f = db.faces().next().unwrap();
            let name = f.families[0].0.clone();
            assert_eq!(usable(&db, f.id), want);
            assert_eq!(broken_family(&db, &name), !want);
        }
        say!("HWPTEST broken font check ok");
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
            let n = page_count(&f);
            let pages = pdf_open(&f).ok().and_then(|doc| unsafe { pdf_page(&doc, 0, 600, 850) });
            let t1 = t.elapsed().as_millis();
            let last = n.and_then(|n| page_pdf(&f, n - 1)).is_some();
            say!("HWPTEST {name}: pages={n:?} first_ms={t1} last_page={last} total_ms={}", t.elapsed().as_millis());
            if let Some(p) = pages {
                let (pct, a) = art(&p);
                say!("HWPTEST page 1 {}x{} dark={pct:.2}%\n{a}", p.w, p.h);
                drawn += (pct > 0.5) as usize;
            }
        }
        // a broken file and a cut-off real one: no page, and no panic out of rhwp
        let junk = d.join("junk.hwp");
        std::fs::write(&junk, (0..70000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect::<Vec<_>>()).unwrap();
        say!("HWPTEST junk: {:?}", page_count(&junk.to_string_lossy()));
        let src = d.join("s0.hwp");
        if let Ok(b) = std::fs::read(&src) {
            let cut = d.join("cut.hwp");
            std::fs::write(&cut, &b[..b.len() / 2]).unwrap();
            say!("HWPTEST cut: {:?}", page_count(&cut.to_string_lossy()));
        }
        assert_eq!(drawn, docs.len(), "every sample's first page has ink");
    }
}
