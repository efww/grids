use crate::imports::*;
use crate::ui::*;
use crate::session::*;
use crate::i18n::*;
use super::docparse::*;
use super::peek::{fix_alpha, PeekReq, Pix};
use windows::Win32::Storage::EnhancedStorage::*;
use windows::Win32::Storage::FileSystem::{FileTimeToLocalFileTime, GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, FILE_FLAGS_AND_ATTRIBUTES};
use windows::Win32::System::Com::StructuredStorage::{IStorage, StgOpenStorageEx, STGFMT_STORAGE};
use windows::Win32::System::Time::FileTimeToSystemTime;
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PSFormatForDisplayAlloc, SHGetPropertyStoreFromParsingName, GPS_DEFAULT, PDFF_DEFAULT};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Storage::StorageFile;
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};

// ---------- what F3 shows for a file: picked and made on the preview worker ----------

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum TextKind {
    Plain,    // wrapped
    Code,     // fixed-width, not wrapped
    Markdown, // headings, bullets and code blocks styled
    Document, // wrapped body with table rows
}

pub(crate) enum Body {
    Pic(Pix),
    Document { pic: Option<Pix>, text: String, cut: bool },
    Pages { pages: Vec<Pix>, total: u32 }, // PDF pages as pictures, the rest added while they render
    Text { text: String, kind: TextKind, cut: bool },
    Table { rows: Vec<Vec<String>>, cut: bool },
    List { head: String, rows: Vec<(String, String)> }, // folder or archive contents: name, size
    Handler(GUID),                                       // a Windows preview handler shows it (on the UI thread)
    Card { icon: Option<Pix>, rows: Vec<(String, String)> },
}

const TEXT_MAX: usize = 512 * 1024;
// Office, PDF, HTML, mail and HWP previews open the whole file (in a preview handler, or here): bigger ones
// get the card instead of a long wait. Pictures, text (its start), archives (their index) have no limit.
pub(crate) const HEAVY_MAX: u64 = 10 << 20;

const PICS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "dib", "tif", "tiff", "webp", "heic", "heif", "avif", "ico", "cur", "jxr", "wdp", "dds",
    "raw", "cr2", "cr3", "nef", "nrw", "arw", "dng", "orf", "rw2", "raf", "srw", "pef",
];
const VIDEOS: &[&str] = &["mp4", "m4v", "mov", "avi", "wmv", "mkv", "webm", "mpg", "mpeg", "ts", "m2ts", "mts", "3gp", "flv", "asf"];
const AUDIO: &[&str] = &["mp3", "wav", "flac", "m4a", "aac", "wma", "ogg", "opus", "aiff", "aif", "mid", "midi"];
// shown by the Windows preview handler when there is one (Office, Outlook, the browser engine, fonts, a PDF reader)
const HANDLED: &[&str] = &[
    "doc", "docx", "docm", "dot", "dotx", "rtf", "odt", "xls", "xlsx", "xlsm", "xlsb", "ods", "ppt", "pptx", "pptm", "pps", "ppsx",
    "odp", "vsd", "vsdx", "msg", "html", "htm", "mht", "mhtml", "xps", "oxps", "ttf", "otf", "ttc", "fon",
];
const CODE: &[&str] = &[
    "json", "jsonc", "json5", "ndjson", "geojson", "xml", "xaml", "xsd", "xsl", "xslt", "svg", "plist", "yaml", "yml", "toml", "ini", "cfg", "conf",
    "config", "properties", "env", "reg", "inf", "rs", "py", "pyw", "ipynb", "js", "mjs", "cjs", "ts", "tsx", "jsx", "vue", "svelte", "java", "kt",
    "kts", "scala", "groovy", "gradle", "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "cs", "fs", "vb", "go", "rb", "php", "pl", "pm", "lua", "r",
    "swift", "m", "mm", "dart", "sh", "bash", "zsh", "fish", "bat", "cmd", "ps1", "psm1", "psd1", "sql", "css", "scss", "sass", "less", "lock",
    "gitignore", "gitattributes", "editorconfig", "dockerfile", "makefile", "cmake", "mk", "tex", "bib", "diff", "patch", "asm", "s", "vbs",
    "ahk", "au3", "nsi", "iss", "csproj", "vcxproj", "sln", "props", "targets", "manifest", "rc", "def", "proto", "graphql", "tf", "hcl",
];
const PLAIN: &[&str] = &["txt", "text", "log", "nfo", "srt", "vtt", "ass", "sub", "lrc", "me", "readme", "1st", "asc", "out", "err"];
const ZIPS: &[&str] = &["zip", "jar", "war", "apk", "aab", "nupkg", "vsix", "whl", "xpi", "crx", "appx", "msix", "kmz", "epub", "cbz", "3mf", "sketch"];
const OFFICE_ZIP: &[&str] = &["docx", "docm", "dotx", "xlsx", "xlsm", "xltx", "pptx", "pptm", "ppsx", "potx", "odt", "ods", "odp"];
const PROGRAMS: &[&str] = &["exe", "dll", "sys", "ocx", "cpl", "scr", "msi", "mui", "drv", "efi"];

pub(crate) fn ext_of(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() => e.to_ascii_lowercase(),
        _ => name.to_ascii_lowercase(), // Makefile, Dockerfile, .gitignore
    }
}

// The registered preview handler of an extension; macro-enabled and show variants borrow their plain twin's.
pub(crate) fn handler_for(ext: &str) -> Option<GUID> {
    const PREVIEW: PCWSTR = w!("{8895b1c6-b41f-4c1c-a562-0d564250836f}");
    let twin = match ext {
        "xlsm" | "xltx" | "xlsb" => "xlsx",
        "docm" | "dotx" => "docx",
        "pptm" | "ppsx" | "potx" | "pps" => "pptx",
        "mhtml" => "mht",
        "oxps" => "xps",
        e => e,
    };
    for e in [ext, twin] {
        let mut buf = [0u16; 64];
        let mut n = buf.len() as u32;
        let hr = unsafe { AssocQueryStringW(ASSOCF_NONE, ASSOCSTR_SHELLEXTENSION, &HSTRING::from(format!(".{e}")), PREVIEW, Some(PWSTR(buf.as_mut_ptr())), &mut n) };
        if hr.is_ok() {
            let s = String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]);
            if let Ok(g) = GUID::try_from(s.trim_matches(['{', '}'])) {
                return Some(g);
            }
        }
    }
    None
}

fn read_head(path: &str, max: usize) -> Option<(Vec<u8>, bool)> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut b = Vec::with_capacity(max.min(len as usize));
    f.take(max as u64).read_to_end(&mut b).ok()?;
    Some((b, len > max as u64))
}

fn text_body(path: &str, kind: TextKind) -> Option<Body> {
    let (b, cut) = read_head(path, TEXT_MAX)?;
    looks_text(&b).then(|| Body::Text { text: decode_text(&b), kind, cut })
}

/// The preview of one file or folder. Never just its icon: what can't be shown becomes a card of facts.
pub(crate) unsafe fn content(req: &PeekReq) -> Body {
    let path = req.path.as_str();
    let meta = std::fs::metadata(path).ok();
    if meta.as_ref().is_some_and(|m| m.is_dir()) {
        return folder_list(path);
    }
    let ext = ext_of(path);
    let e = ext.as_str();
    if PICS.contains(&e) {
        return match super::imgjob::isolated_pic(path, req.w, req.h, SIIGBF_RESIZETOFIT.0 as u32) {
            Ok(p) => Body::Pic(p),
            Err(msg) => image_fail(path, &msg),
        };
    }
    if VIDEOS.contains(&e) {
        return match super::imgjob::isolated_pic(path, req.w, req.h, (SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY).0 as u32) {
            Ok(p) => Body::Pic(p),
            Err(msg) => image_fail(path, &msg),
        };
    }
    if AUDIO.contains(&e) {
        return card(path, &[]);
    }
    let heavy = HANDLED.contains(&e) || OFFICE_ZIP.contains(&e) || matches!(e, "pdf" | "eml" | "hwp" | "hwpx" | "hwpt" | "psd" | "psb");
    if heavy && meta.as_ref().is_some_and(|m| m.len() > HEAVY_MAX) {
        let note = (tr(crate::Key::Preview).to_string(), tr(crate::Key::SkippedLargerThan10Mb).to_string());
        return card(path, &[note]);
    }
    if !req.plain && HANDLED.contains(&e) {
        if let Some(g) = handler_for(e) {
            return Body::Handler(g);
        }
    }
    let found = match e {
        "pdf" => pdf_first(path, req).or_else(|| (!req.plain).then(|| handler_for(e).map(Body::Handler)).flatten()),
        "csv" | "tsv" => read_head(path, TEXT_MAX).filter(|(b, _)| looks_text(b)).map(|(b, cut)| {
            let text = decode_text(&b);
            let sep = if e == "tsv" { '\t' } else { csv_sep(&text) };
            Body::Table { rows: csv_rows(&text, sep, 2000), cut }
        }),
        "md" | "markdown" | "mdown" | "mkd" => text_body(path, TextKind::Markdown),
        e if PLAIN.contains(&e) => text_body(path, TextKind::Plain),
        e if CODE.contains(&e) => text_body(path, TextKind::Code),
        // A registered handler is tried above. With none, or after it fails (`plain`), show the source.
        "html" | "htm" | "xhtml" => text_body(path, TextKind::Code),
        "eml" => read_head(path, 4 << 20).map(|(b, cut)| Body::Text { text: eml_text(&b), kind: TextKind::Plain, cut }),
        "url" | "website" => url_card(path),
        "lnk" => Some(lnk_card(path)),
        "hwp" => hwp_preview(path, req),
        "hwpx" | "hwpt" => hwpx_preview(path, req),
        e if OFFICE_ZIP.contains(&e) => office_text(path, e),
        e if ZIPS.contains(&e) => zip_list(path),
        e if PROGRAMS.contains(&e) => Some(card(path, &version_rows(path))),
        "psd" | "psb" => psd_thumb(path, req).map(Body::Pic),
        _ => None,
    };
    if let Some(b) = found {
        return b;
    }
    // unknown type: a thumbnail from a child process, text if it reads as text, else the facts
    if let Ok(p) = super::imgjob::isolated_pic(path, req.w, req.h, (SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY).0 as u32) {
        return Body::Pic(p);
    }
    if !PROGRAMS.contains(&e) && meta.as_ref().is_some_and(|m| m.len() > 0) {
        if let Some(b) = text_body(path, TextKind::Plain) {
            return b;
        }
    }
    card(path, &[])
}

// ---------- pictures ----------

/// The shell's picture of an item at about w x h: its thumbnail, or with SIIGBF_ICONONLY its icon.
pub(crate) unsafe fn shell_pic(path: &str, w: i32, h: i32, flags: SIIGBF) -> Option<Pix> {
    let f: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
    let bmp = f.GetImage(SIZE { cx: w, cy: h }, flags).ok()?;
    let p = bitmap_pix(bmp);
    _ = DeleteObject(bmp.into());
    p
}

fn checked_pix(w: i32, h: i32) -> Option<usize> {
    if !(1..=16_384).contains(&w) || !(1..=16_384).contains(&h) {
        return None;
    }
    (w as usize).checked_mul(h as usize)?.checked_mul(4)
}

pub(crate) unsafe fn bitmap_pix(bmp: HBITMAP) -> Option<Pix> {
    let mut bm = BITMAP::default();
    GetObjectW(bmp.into(), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _));
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    let nbytes = checked_pix(w, h)?;
    let mut bgra = vec![0u8; nbytes];
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let dc = CreateCompatibleDC(None);
    let rows = if bgra.is_empty() { 0 } else { GetDIBits(dc, bmp, 0, h as u32, Some(bgra.as_mut_ptr() as _), &mut bmi, DIB_RGB_COLORS) };
    _ = DeleteDC(dc);
    if rows != h || rows == 0 {
        return None;
    }
    fix_alpha(&mut bgra);
    Some(Pix::new(w, h, bgra))
}

/// Picture bytes (PNG, JPEG, GIF, BMP ...) decoded by WIC, shrunk to fit w x h.
pub(crate) unsafe fn decode_pic(bytes: &[u8], w: i32, h: i32) -> Option<Pix> {
    let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
    let stream = SHCreateMemStream(Some(bytes))?;
    let frame = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?.GetFrame(0).ok()?;
    let (mut sw, mut sh) = (0u32, 0u32);
    frame.GetSize(&mut sw, &mut sh).ok()?;
    if sw == 0 || sh == 0 || sw > 16_384 || sh > 16_384 {
        return None;
    }
    let k = (w.max(1) as f64 / sw as f64).min(h.max(1) as f64 / sh as f64).min(1.0);
    let (mut dw, mut dh) = (((sw as f64 * k) as u32).max(1), ((sh as f64 * k) as u32).max(1));
    while dw.saturating_mul(dh) > 16_000_000 {
        dw = (dw / 2).max(1);
        dh = (dh / 2).max(1);
        if dw == 1 && dh == 1 {
            break;
        }
    }
    let src: IWICBitmapSource = if (dw, dh) != (sw, sh) {
        let sc = wic.CreateBitmapScaler().ok()?;
        sc.Initialize(&frame, dw, dh, WICBitmapInterpolationModeFant).ok()?;
        sc.cast().ok()?
    } else {
        frame.cast().ok()?
    };
    let conv = wic.CreateFormatConverter().ok()?;
    conv.Initialize(&src, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
    let nbytes = (dw as usize).checked_mul(dh as usize)?.checked_mul(4)?;
    let stride = dw.checked_mul(4)?;
    let mut bgra = vec![0u8; nbytes];
    conv.CopyPixels(std::ptr::null(), stride, &mut bgra).ok()?;
    Some(Pix::new(dw as i32, dh as i32, bgra))
}

// Photoshop keeps a small JPEG (image resource 1036) for file browsers.
unsafe fn psd_thumb(path: &str, req: &PeekReq) -> Option<Pix> {
    let (b, _) = read_head(path, 8 << 20)?;
    let be32 = |at: usize| b.get(at..at + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]) as usize);
    if !b.starts_with(b"8BPS") {
        return None;
    }
    let res = 26usize.checked_add(4)?.checked_add(be32(26)?)?;
    let mut p = res.checked_add(4)?;
    let stop = p.checked_add(be32(res)?).unwrap_or(b.len()).min(b.len());
    while p + 12 <= stop && b.get(p..p + 4) == Some(b"8BIM") {
        let id = u16::from_be_bytes([b[p + 4], b[p + 5]]);
        let mut q = p + 6 + 1 + b[p + 6] as usize;
        q += q % 2; // the Pascal name is padded to even
        let size = be32(q)?;
        let data = b.get(q + 4..q.checked_add(4)?.checked_add(size)?)?;
        if (id == 1036 || id == 1033) && data.len() > 28 {
            return decode_pic(&data[28..], req.w, req.h);
        }
        let Some(next) = q.checked_add(4).and_then(|n| n.checked_add(size)).and_then(|n| n.checked_add(size % 2)) else { break };
        if next <= p || next > b.len() {
            break;
        }
        p = next;
    }
    None
}

// ---------- PDF: Windows' own renderer, pages as pictures ----------

pub(crate) const PDF_MAX: u32 = 50;

pub(crate) fn pdf_open(path: &str) -> Result<PdfDocument> {
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))?.join()?;
    PdfDocument::LoadFromFileAsync(&file)?.join()
}

/// Pixel size of a page fitted into `w`×`h`. Width is what `pdf_page` asks the renderer for.
pub(crate) fn pdf_fit_px(page_w: f32, page_h: f32, w: i32, h: i32) -> (i32, i32) {
    let page_h = if page_h.is_finite() && page_h > 0.0 { page_h } else { 1.0 };
    let page_w = if page_w.is_finite() { page_w.max(0.0) } else { 0.0 };
    let fit = (w.max(0) as f32).min(h.max(0) as f32 * page_w / page_h).max(16.0);
    let dw = fit as i32;
    let dh = (fit * page_h / page_w.max(1.0)).max(1.0) as i32;
    (dw.max(1), dh.max(1))
}

/// Page i fitted whole into w x h (px), white paper as the reader would show it.
pub(crate) unsafe fn pdf_page(doc: &PdfDocument, i: u32, w: i32, h: i32) -> Option<Pix> {
    let page = doc.GetPage(i).ok()?;
    let size = page.Size().ok()?;
    let dw = pdf_fit_px(size.Width, size.Height, w, h).0 as u32;
    let opts = PdfPageRenderOptions::new().ok()?;
    opts.SetDestinationWidth(dw).ok()?;
    let st = InMemoryRandomAccessStream::new().ok()?;
    page.RenderWithOptionsToStreamAsync(&st, &opts).ok()?.join().ok()?;
    let n = st.Size().ok()? as u32;
    let rd = DataReader::CreateDataReader(&st.GetInputStreamAt(0).ok()?).ok()?;
    rd.LoadAsync(n).ok()?.join().ok()?;
    let mut buf = vec![0u8; n as usize];
    rd.ReadBytes(&mut buf).ok()?;
    decode_pic(&buf, w, i32::MAX / 4)
}

// The first page now; peek_worker renders the others after it is on screen (pdf_rest).
unsafe fn pdf_first(path: &str, req: &PeekReq) -> Option<Body> {
    let doc = pdf_open(path).ok()?;
    let total = doc.PageCount().ok()?.min(PDF_MAX);
    let first = pdf_page(&doc, 0, req.w, req.h)?;
    Some(Body::Pages { pages: vec![first], total })
}

// ---------- the header: icon, name, type, size, dates, attributes, and image facts ----------

pub(crate) const FA_READONLY: u32 = 0x1;
pub(crate) const FA_HIDDEN: u32 = 0x2;
pub(crate) const FA_SYSTEM: u32 = 0x4;
pub(crate) const FA_ARCHIVE: u32 = 0x20;
pub(crate) const FA_TEMPORARY: u32 = 0x100;
pub(crate) const FA_SPARSE: u32 = 0x200;
pub(crate) const FA_COMPRESSED: u32 = 0x800;
pub(crate) const FA_OFFLINE: u32 = 0x1000;
pub(crate) const FA_ENCRYPTED: u32 = 0x4000;

/// Labels for [`format_attributes`]. Empty strings are skipped.
pub(crate) struct AttrWords<'a> {
    pub read_only: &'a str,
    pub hidden: &'a str,
    pub system: &'a str,
    pub archive: &'a str,
    pub compressed: &'a str,
    pub encrypted: &'a str,
    pub offline: &'a str,
    pub temporary: &'a str,
    pub sparse: &'a str,
}

/// Read-only, hidden, system, archive, and the other bits worth naming. Directory and Normal are left out.
pub(crate) fn format_attributes(bits: u32, w: &AttrWords) -> String {
    let named = [
        (FA_READONLY, w.read_only),
        (FA_HIDDEN, w.hidden),
        (FA_SYSTEM, w.system),
        (FA_ARCHIVE, w.archive),
        (FA_COMPRESSED, w.compressed),
        (FA_ENCRYPTED, w.encrypted),
        (FA_OFFLINE, w.offline),
        (FA_TEMPORARY, w.temporary),
        (FA_SPARSE, w.sparse),
    ];
    named.into_iter().filter(|(bit, name)| bits & bit != 0 && !name.is_empty()).map(|(_, name)| name).collect::<Vec<_>>().join(", ")
}

/// Raw image properties. Empty options are hidden by [`format_exif`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExifIn {
    pub make: Option<String>,
    pub model: Option<String>,
    pub taken: Option<String>,
    pub exposure: Option<String>,
    pub fnumber: Option<String>,
    pub iso: Option<String>,
    pub focal: Option<String>,
    pub latitude: Option<String>,
    pub longitude: Option<String>,
}

pub(crate) struct ExifLabels<'a> {
    pub camera: &'a str,
    pub taken: &'a str,
    pub exposure: &'a str,
    pub aperture: &'a str,
    pub iso: &'a str,
    pub focal: &'a str,
    pub gps: &'a str,
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.and_then(|v| {
        let t = v.trim().to_string();
        (!t.is_empty()).then_some(t)
    })
}

/// Camera, exposure and GPS lines. A missing value is omitted. Make and model share one line.
pub(crate) fn format_exif(v: &ExifIn, labels: &ExifLabels) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let make = nonempty(v.make.clone());
    let model = nonempty(v.model.clone());
    let camera = match (make, model) {
        (Some(a), Some(b)) if a != b => Some(format!("{a} {b}")),
        (Some(a), _) => Some(a),
        (_, Some(b)) => Some(b),
        _ => None,
    };
    let lat = nonempty(v.latitude.clone());
    let lon = nonempty(v.longitude.clone());
    let gps = match (lat, lon) {
        (Some(a), Some(b)) => Some(format!("{a}, {b}")),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        _ => None,
    };
    let mut push = |label: &str, value: Option<String>| {
        if let Some(value) = nonempty(value) {
            if !label.is_empty() {
                out.push((label.to_string(), value));
            }
        }
    };
    push(labels.camera, camera);
    push(labels.taken, v.taken.clone());
    push(labels.exposure, v.exposure.clone());
    push(labels.aperture, v.fnumber.clone());
    push(labels.iso, v.iso.clone());
    push(labels.focal, v.focal.clone());
    push(labels.gps, gps);
    out
}

/// One logical header field, already worded. Empty fields should not be passed in.
pub(crate) fn ellipsize(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if max_chars == 0 || n <= max_chars {
        return s.to_string();
    }
    if max_chars == 1 {
        return "…".into();
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

/// Pack ` · `-separated fields onto lines of at most `max_chars`. A single field that is
/// still too long is ellipsized. Empty fields are skipped.
pub(crate) fn wrap_fields(fields: &[String], max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(4);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for f in fields {
        if f.is_empty() {
            continue;
        }
        let piece = ellipsize(f, max_chars);
        if cur.is_empty() {
            cur = piece;
            continue;
        }
        let joined = format!("{cur} · {piece}");
        if joined.chars().count() <= max_chars {
            cur = joined;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = piece;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Name on the first line, then the attribute fields wrapped to `max_chars`.
pub(crate) fn header_lines(name: &str, fields: &[String], max_chars: usize) -> Vec<String> {
    let mut lines = vec![ellipsize(name, max_chars.max(4))];
    lines.extend(wrap_fields(fields, max_chars));
    lines
}

/// How many characters a header line can hold. `fh` is the UI font height in px.
pub(crate) fn header_char_budget(width_px: i32, fh: i32) -> usize {
    let cw = (fh.max(1) * 3 / 5).max(6);
    (width_px.max(0) / cw).max(8) as usize
}

pub(crate) fn exif_line(pairs: &[(String, String)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(" · ")
}

/// Facts drawn in the attribute header. `facts` is the short list Quick Look used to join.
#[derive(Clone, Debug)]
pub(crate) struct FileFacts {
    pub icon: i32,
    pub name: String,
    pub type_name: String,
    pub size: String,
    pub modified: String,
    pub created: String,
    pub attributes: String,
    pub dimensions: String,
    pub extra: String,
    pub exif: Vec<(String, String)>,
    pub facts: Vec<String>,
}

impl Default for FileFacts {
    fn default() -> Self {
        Self {
            icon: -1,
            name: String::new(),
            type_name: String::new(),
            size: String::new(),
            modified: String::new(),
            created: String::new(),
            attributes: String::new(),
            dimensions: String::new(),
            extra: String::new(),
            exif: Vec::new(),
            facts: Vec::new(),
        }
    }
}

impl FileFacts {
    /// Non-empty attribute fields, in header order. The name is separate.
    pub(crate) fn fields(&self) -> Vec<String> {
        let mut v = Vec::new();
        for s in [&self.type_name, &self.size, &self.dimensions, &self.extra, &self.modified, &self.created, &self.attributes] {
            if !s.is_empty() {
                v.push(s.clone());
            }
        }
        v
    }
}

unsafe fn prop(ps: &IPropertyStore, k: &PROPERTYKEY) -> Option<String> {
    let v = ps.GetValue(k).ok()?;
    let s = pwstr(PSFormatForDisplayAlloc(k, &v, PDFF_DEFAULT).ok()?);
    (!s.trim().is_empty()).then_some(s)
}

fn photo_key(pid: u32) -> PROPERTYKEY {
    PROPERTYKEY { fmtid: GUID::from_values(0x14B81DA1, 0x0135, 0x4D31, [0x96, 0xD9, 0x6C, 0xBF, 0xC9, 0x67, 0x1A, 0x99]), pid }
}

fn gps_key(pid: u32) -> PROPERTYKEY {
    PROPERTYKEY { fmtid: GUID::from_values(0x8727CFFF, 0x4868, 0x4EC6, [0xAD, 0x5B, 0x81, 0xB6, 0x60, 0xC6, 0x77, 0xD8]), pid }
}

fn exif_labels() -> ExifLabels<'static> {
    ExifLabels {
        camera: tr(crate::Key::ExifCamera),
        taken: tr(crate::Key::ExifTaken),
        exposure: tr(crate::Key::ExifExposure),
        aperture: tr(crate::Key::ExifAperture),
        iso: tr(crate::Key::ExifIso),
        focal: tr(crate::Key::ExifFocal),
        gps: tr(crate::Key::ExifGps),
    }
}

fn attr_words() -> AttrWords<'static> {
    AttrWords {
        read_only: tr(crate::Key::AttrReadOnly),
        hidden: tr(crate::Key::AttrHidden),
        system: tr(crate::Key::AttrSystem),
        archive: tr(crate::Key::AttrArchive),
        compressed: tr(crate::Key::AttrCompressed),
        encrypted: tr(crate::Key::AttrEncrypted),
        offline: tr(crate::Key::AttrOffline),
        temporary: tr(crate::Key::AttrTemporary),
        sparse: tr(crate::Key::AttrSparse),
    }
}

unsafe fn read_exif(ps: &IPropertyStore) -> ExifIn {
    ExifIn {
        make: prop(ps, &photo_key(271)),
        model: prop(ps, &photo_key(272)),
        taken: prop(ps, &photo_key(36867)),
        exposure: prop(ps, &photo_key(33434)),
        fnumber: prop(ps, &photo_key(33437)),
        iso: prop(ps, &photo_key(34855)),
        focal: prop(ps, &photo_key(37386)),
        latitude: prop(ps, &gps_key(100)),
        longitude: prop(ps, &gps_key(101)),
    }
}

fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

/// Icon and attribute header for `path`. `body` adds dimensions, page counts and similar facts.
/// Pass `None` while a preview handler is showing and those counts are not known yet.
pub(crate) unsafe fn meta(path: &str, body: Option<&Body>) -> FileFacts {
    let mut fi = SHFILEINFOW::default();
    let flags = SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_TYPENAME;
    let got = SHGetFileInfoW(&HSTRING::from(path), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut fi), size_of::<SHFILEINFOW>() as u32, flags);
    let type_name = String::from_utf16_lossy(&fi.szTypeName[..fi.szTypeName.iter().position(|&c| c == 0).unwrap_or(0)]);
    let mut facts = FileFacts { icon: if got == 0 { -1 } else { fi.iIcon }, name: file_name(path), type_name: type_name.clone(), ..FileFacts::default() };
    let mut out = Vec::new();
    if !type_name.is_empty() {
        out.push(type_name);
    }
    let e = ext_of(path);
    let ps = SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(&HSTRING::from(path), None, GPS_DEFAULT).ok();
    let image = PICS.contains(&e.as_str());
    if let Some(body) = body {
        match body {
            Body::Pic(_) if VIDEOS.contains(&e.as_str()) => {
                if let Some(ps) = &ps {
                    let (w, h) = (prop(ps, &PKEY_Video_FrameWidth), prop(ps, &PKEY_Video_FrameHeight));
                    if let (Some(w), Some(h)) = (w, h) {
                        let dim = format!("{w} × {h}");
                        facts.dimensions = dim.clone();
                        out.push(dim);
                    }
                    if let Some(d) = prop(ps, &PKEY_Media_Duration) {
                        facts.extra = d.clone();
                        out.push(d);
                    }
                }
            }
            Body::Pic(_) => {
                if let Some(d) = ps.as_ref().and_then(|ps| prop(ps, &PKEY_Image_Dimensions)).map(|d| d.replace(" x ", " × ")) {
                    facts.dimensions = d.clone();
                    out.push(d);
                }
            }
            Body::Pages { total, .. } => {
                let s = tf(crate::Key::PagesN, &[("total", &total.to_string())]);
                facts.extra = s.clone();
                out.push(s);
            }
            Body::Text { text, .. } | Body::Document { text, .. } => {
                let s = tf(crate::Key::LinesN, &[("n", &text.lines().count().to_string())]);
                facts.extra = s.clone();
                out.push(s);
            }
            Body::Table { rows, .. } => {
                let s = tf(crate::Key::RowsN, &[("n", &rows.len().to_string())]);
                facts.extra = s.clone();
                out.push(s);
            }
            Body::List { head, .. } => {
                facts.extra = head.clone();
                out.push(head.clone());
            }
            _ => {}
        }
    } else if image {
        if let Some(d) = ps.as_ref().and_then(|ps| prop(ps, &PKEY_Image_Dimensions)).map(|d| d.replace(" x ", " × ")) {
            facts.dimensions = d.clone();
            out.push(d);
        }
    }
    if image {
        if let Some(ps) = &ps {
            facts.exif = format_exif(&read_exif(ps), &exif_labels());
        }
    }
    if let Ok(m) = std::fs::metadata(path) {
        if m.is_file() {
            let s = human(m.len());
            facts.size = s.clone();
            out.push(s);
        }
        if let Some(t) = when(m.modified()) {
            facts.modified = format!("{} {t}", tr(crate::Key::Modified));
            out.push(t);
        }
        if let Some(t) = when(m.created()) {
            facts.created = format!("{} {t}", tr(crate::Key::Created));
        }
        use std::os::windows::fs::MetadataExt;
        facts.attributes = format_attributes(m.file_attributes(), &attr_words());
    }
    facts.facts = out;
    facts
}

// ---------- lists ----------

fn folder_list(path: &str) -> Body {
    let Ok(rd) = std::fs::read_dir(path) else {
        return Body::List { head: tr(crate::Key::CanTOpen).into(), rows: Vec::new() };
    };
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        match e.metadata() {
            Ok(m) if m.is_dir() => dirs.push(name),
            Ok(m) => files.push((name, m.len())),
            Err(_) => files.push((name, 0)),
        }
    }
    dirs.sort_by_key(|n| n.to_lowercase());
    files.sort_by_key(|(n, _)| n.to_lowercase());
    let total: u64 = files.iter().map(|(_, s)| s).sum();
    let head = tf(crate::Key::FolderListHead, &[("dirs", &dirs.len().to_string()), ("files", &files.len().to_string()), ("size", &human(total))]);
    let folder = tr(crate::Key::Folder);
    let rows = dirs.into_iter().map(|d| (d, folder.to_string())).chain(files.into_iter().map(|(n, s)| (n, human(s)))).take(2000).collect();
    Body::List { head, rows }
}

fn zip_list(path: &str) -> Option<Body> {
    let mut f = std::fs::File::open(path).ok()?;
    let es = zip_entries(&mut f)?;
    let files: Vec<&ZipEntry> = es.iter().filter(|e| !e.is_dir()).collect();
    let total: u64 = files.iter().map(|e| e.size).sum();
    let head = tf(crate::Key::ZipListHead, &[("files", &files.len().to_string()), ("size", &human(total))]);
    Some(Body::List { head, rows: files.iter().take(5000).map(|e| (e.name.clone(), human(e.size))).collect() })
}

// ---------- documents read without their programs ----------

/// Word, Excel, PowerPoint and OpenDocument files as their words (the second best, when no preview handler).
fn office_text(path: &str, ext: &str) -> Option<Body> {
    let mut f = std::fs::File::open(path).ok()?;
    let es = zip_entries(&mut f)?;
    let mut part = |name: &str| zip_find(&es, name).and_then(|e| zip_read(&mut f, e, 32 << 20)).map(|b| decode_text(&b));
    let text = |xml: String, t: &[&str], p: &[&str]| xml_text(&xml, t, p, &["w:tab", "a:tab", "text:tab"], &["w:br", "a:br", "text:line-break"]);
    let body = match ext {
        "docx" | "docm" | "dotx" => text(part("word/document.xml")?, &["w:t"], &["w:p"]),
        "xlsx" | "xlsm" | "xltx" => {
            let strings = part("xl/sharedStrings.xml").map(|x| xlsx_strings(&x)).unwrap_or_default();
            let sheet = part("xl/worksheets/sheet1.xml").or_else(|| {
                let first = es.iter().find(|e| e.name.starts_with("xl/worksheets/sheet") && e.name.ends_with(".xml"))?.name.clone();
                part(&first)
            })?;
            let rows = xlsx_rows(&sheet, &strings, 2000, 60);
            return Some(Body::Table { rows, cut: false });
        }
        "pptx" | "pptm" | "ppsx" | "potx" => {
            let mut slides: Vec<(usize, String)> = es
                .iter()
                .filter_map(|e| {
                    let n = e.name.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()?;
                    Some((n, e.name.clone()))
                })
                .collect();
            slides.sort();
            let label = tr(crate::Key::Slide);
            let mut out = String::new();
            for (n, name) in slides.iter().take(300) {
                let t = text(part(name).unwrap_or_default(), &["a:t"], &["a:p"]);
                out += &format!("── {label} {n} ──\n{}\n\n", t.trim());
            }
            out
        }
        _ => text(part("content.xml")?, &["text:p", "text:h"], &["text:p", "text:h"]),
    };
    Some(Body::Text { text: body.trim().to_string(), kind: TextKind::Plain, cut: false })
}

// HWPX: retain the preview image, followed by the body of every section.
unsafe fn hwpx_preview(path: &str, req: &PeekReq) -> Option<Body> {
    let mut f = std::fs::File::open(path).ok()?;
    let es = zip_entries(&mut f)?;
    let pic = zip_find(&es, "Preview/PrvImage.png")
        .and_then(|e| zip_read(&mut f, e, 16 << 20))
        .and_then(|b| decode_pic(&b, req.w, req.h));
    let mut sections: Vec<&ZipEntry> = es
        .iter()
        .filter(|e| e.name.starts_with("Contents/section") && e.name.ends_with(".xml"))
        .collect();
    sections.sort_by_key(|e| {
        e.name
            .trim_start_matches("Contents/section")
            .trim_end_matches(".xml")
            .parse::<u32>()
            .unwrap_or(u32::MAX)
    });
    let mut out = String::new();
    let mut cut = false;
    for e in sections {
        if out.len() >= TEXT_MAX {
            cut = true;
            break;
        }
        if e.size > 32 << 20 {
            cut = true;
        }
        if let Some(b) = zip_read(&mut f, e, 32 << 20) {
            out.push_str(&hwpx_body(&decode_text(&b)));
        }
    }
    if out.trim().is_empty() {
        out = zip_find(&es, "Preview/PrvText.txt")
            .and_then(|e| zip_read(&mut f, e, TEXT_MAX))
            .map(|b| decode_text(&b))
            .unwrap_or_default();
    }
    limit_document(&mut out, &mut cut);
    if pic.is_none() && out.is_empty() {
        None
    } else {
        Some(Body::Document { pic, text: out, cut })
    }
}

fn limit_document(out: &mut String, cut: &mut bool) {
    if out.len() > TEXT_MAX {
        let mut n = TEXT_MAX;
        while !out.is_char_boundary(n) {
            n -= 1;
        }
        out.truncate(n);
        *cut = true;
    }
}

// HWP5: OLE streams are read through the existing Windows structured storage API.
unsafe fn hwp_preview(path: &str, req: &PeekReq) -> Option<Body> {
    let name = HSTRING::from(path);
    let mut stg: Option<IStorage> = None;
    for mode in [STGM_READ | STGM_SHARE_DENY_WRITE, STGM_READ | STGM_SHARE_DENY_NONE | STGM_TRANSACTED] {
        let mut p: *mut std::ffi::c_void = std::ptr::null_mut();
        if StgOpenStorageEx(&name, mode, STGFMT_STORAGE, 0, None, None, &IStorage::IID, &mut p).is_ok() && !p.is_null() {
            stg = Some(IStorage::from_raw(p));
            break;
        }
    }
    let stg = stg?;
    let read = |storage: &IStorage, s: &str| -> Option<Vec<u8>> {
        let st = storage.OpenStream(&HSTRING::from(s), None, STGM_READ | STGM_SHARE_EXCLUSIVE, 0).ok()?;
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut n = 0u32;
            if st.Read(buf.as_mut_ptr() as _, buf.len() as u32, Some(&mut n)).is_err() || n == 0 || out.len() >= 16 << 20 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        (!out.is_empty()).then_some(out)
    };
    let pic = read(&stg, "PrvImage").and_then(|b| decode_pic(&b, req.w, req.h));
    let flags = read(&stg, "FileHeader").filter(|b| b.len() >= 40).map(|b| u32::from_le_bytes(b[36..40].try_into().unwrap()));
    if flags.is_none_or(|f| f & 6 != 0) {
        if let Some(p) = pic {
            return Some(Body::Pic(p));
        }
        return read(&stg, "PrvText").map(|b| Body::Text {
            text: utf16(&b, false).trim_end_matches('\0').to_string(),
            kind: TextKind::Plain,
            cut: false,
        });
    }
    let mut out = String::new();
    let mut cut = false;
    if let Some(flags) = flags {
        if let Ok(body) = stg.OpenStorage(&HSTRING::from("BodyText"), None, STGM_READ | STGM_SHARE_EXCLUSIVE, std::ptr::null(), 0) {
            for n in 0..1000 {
                if out.len() >= TEXT_MAX {
                    cut = true;
                    break;
                }
                let Some(data) = read(&body, &format!("Section{n}")) else { break };
                let data = if flags & 1 != 0 {
                    match miniz_oxide::inflate::decompress_to_vec_with_limit(&data, 32 << 20) {
                        Ok(v) => v,
                        Err(e) if !e.output.is_empty() => {
                            cut = true;
                            e.output
                        }
                        Err(_) => break,
                    }
                } else {
                    data
                };
                out.push_str(&hwp5_body(&data));
            }
        }
    }
    if out.trim().is_empty() {
        out = read(&stg, "PrvText").map(|b| utf16(&b, false).trim_end_matches('\0').to_string()).unwrap_or_default();
    }
    limit_document(&mut out, &mut cut);
    if pic.is_none() && out.is_empty() {
        None
    } else {
        Some(Body::Document { pic, text: out, cut })
    }
}

// ---------- cards: the facts, with the big icon ----------

fn when(t: std::io::Result<std::time::SystemTime>) -> Option<String> {
    let t = t.ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    let ft = (t.as_secs() + 11_644_473_600) * 10_000_000 + t.subsec_nanos() as u64 / 100;
    let ft = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let mut local = FILETIME::default();
    let mut st = SYSTEMTIME::default();
    unsafe {
        FileTimeToLocalFileTime(&ft, &mut local).ok()?;
        FileTimeToSystemTime(&local, &mut st).ok()?;
    }
    Some(format!("{:04}-{:02}-{:02} {:02}:{:02}", st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute))
}

/// 30360942 -> "30,360,942"
pub(crate) fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (k, c) in s.chars().enumerate() {
        if k > 0 && (s.len() - k) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// Media and document properties Windows knows, in the shell's own wording ("3:25", "320kbps").
unsafe fn props(path: &str) -> Vec<(String, String)> {
    let Ok(ps) = SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(&HSTRING::from(path), None, GPS_DEFAULT) else { return Vec::new() };
    let keys: [(crate::Key, PROPERTYKEY); 8] = [
        (crate::Key::PropTitle, PKEY_Title),
        (crate::Key::PropArtist, PKEY_Music_Artist),
        (crate::Key::PropAlbum, PKEY_Music_AlbumTitle),
        (crate::Key::PropLength, PKEY_Media_Duration),
        (crate::Key::PropBitrate, PKEY_Audio_EncodingBitrate),
        (crate::Key::PropDimensions, PKEY_Image_Dimensions),
        (crate::Key::PropAuthor, PKEY_Author),
        (crate::Key::PropPages, PKEY_Document_PageCount),
    ];
    let mut out = Vec::new();
    for (label, k) in keys {
        let Ok(v) = ps.GetValue(&k) else { continue };
        if let Ok(s) = PSFormatForDisplayAlloc(&k, &v, PDFF_DEFAULT) {
            let s = pwstr(s);
            if !s.trim().is_empty() {
                out.push((tr(label).to_string(), s));
            }
        }
    }
    out
}

fn image_fail(path: &str, msg: &str) -> Body {
    let mut rows = vec![(tr(crate::Key::Preview).to_string(), msg.to_string())];
    if let Ok(m) = std::fs::metadata(path) {
        rows.push((tr(crate::Key::Size).into(), format!("{} B", group_digits(m.len()))));
    }
    Body::Card { icon: None, rows }
}

pub(crate) unsafe fn card(path: &str, extra: &[(String, String)]) -> Body {
    // a real thumbnail (album art, a document's first page) may be larger; an icon comes at the size it is drawn
    let dpi = GetDpiForSystem() as i32;
    let (thumb, icon) = (160 * dpi / 96, 96 * dpi / 96);
    let icon = shell_pic(path, thumb, thumb, SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY).or_else(|| shell_pic(path, icon, icon, SIIGBF_ICONONLY));
    // the header line already gives type, size and date: the card has the rest
    let mut rows: Vec<(String, String)> = extra.to_vec();
    rows.extend(props(path));
    if let Ok(m) = std::fs::metadata(path) {
        rows.push((tr(crate::Key::Size).into(), format!("{} B", group_digits(m.len()))));
        if let Some(t) = when(m.created()) {
            rows.push((tr(crate::Key::Created).into(), t));
        }
    }
    if let Some(dir) = Path::new(path).parent() {
        rows.push((tr(crate::Key::Folder2).into(), dir.display().to_string()));
    }
    Body::Card { icon, rows }
}

fn url_card(path: &str) -> Option<Body> {
    let (b, _) = read_head(path, 64 * 1024)?;
    let text = decode_text(&b);
    let url = text.lines().find_map(|l| l.trim().strip_prefix("URL=").map(str::to_string))?;
    Some(unsafe { card(path, &[("URL".into(), url)]) })
}

unsafe fn lnk_card(path: &str) -> Body {
    let mut rows = Vec::new();
    let link: Result<IShellLinkW> = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER);
    if let Ok(link) = link {
        if link.cast::<IPersistFile>().and_then(|pf| pf.Load(&HSTRING::from(path), STGM_READ)).is_ok() {
            let mut buf = [0u16; 1024];
            let s = |b: &[u16]| String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]);
            if link.GetPath(&mut buf, std::ptr::null_mut(), 0).is_ok() && buf[0] != 0 {
                rows.push((tr(crate::Key::Target).into(), s(&buf)));
            }
            buf[0] = 0;
            if link.GetArguments(&mut buf).is_ok() && buf[0] != 0 {
                rows.push((tr(crate::Key::Arguments).into(), s(&buf)));
            }
            buf[0] = 0;
            if link.GetWorkingDirectory(&mut buf).is_ok() && buf[0] != 0 {
                rows.push((tr(crate::Key::StartIn).into(), s(&buf)));
            }
            buf[0] = 0;
            if link.GetDescription(&mut buf).is_ok() && buf[0] != 0 {
                rows.push((tr(crate::Key::Comment).into(), s(&buf)));
            }
        }
    }
    card(path, &rows)
}

/// FileDescription, ProductName, CompanyName, FileVersion of a program, in its first language.
pub(crate) unsafe fn version_rows(path: &str) -> Vec<(String, String)> {
    let name = HSTRING::from(path);
    let n = GetFileVersionInfoSizeW(&name, None);
    if n == 0 {
        return Vec::new();
    }
    let mut data = vec![0u8; n as usize];
    if GetFileVersionInfoW(&name, None, n, data.as_mut_ptr() as _).is_err() {
        return Vec::new();
    }
    let query = |sub: &str| -> Option<(*const u8, u32)> {
        let mut p: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        VerQueryValueW(data.as_ptr() as _, &HSTRING::from(sub), &mut p, &mut len).as_bool().then_some((p as *const u8, len))
    };
    let lang = match query("\\VarFileInfo\\Translation") {
        Some((p, len)) if len >= 4 => {
            let w = std::slice::from_raw_parts(p as *const u16, 2);
            format!("{:04x}{:04x}", w[0], w[1])
        }
        _ => "040904b0".into(),
    };
    let mut rows = Vec::new();
    for (key, label) in [
        ("FileDescription", crate::Key::VerDescription),
        ("ProductName", crate::Key::VerProduct),
        ("CompanyName", crate::Key::VerCompany),
        ("FileVersion", crate::Key::VerVersion),
    ] {
        if let Some((p, len)) = query(&format!("\\StringFileInfo\\{lang}\\{key}")).filter(|(_, l)| *l > 1) {
            let v = String::from_utf16_lossy(std::slice::from_raw_parts(p as *const u16, len as usize - 1));
            if !v.trim().is_empty() {
                rows.push((tr(label).to_string(), v.trim().to_string()));
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_of_a_path() {
        assert_eq!(ext_of(r"C:\a\b.TXT"), "txt");
        assert_eq!(ext_of(r"C:\a\Makefile"), "makefile");
        assert_eq!(ext_of(r"C:\a\.gitignore"), ".gitignore");
    }

    #[test]
    fn grouped_digits() {
        assert_eq!(group_digits(30360942), "30,360,942");
        assert_eq!(group_digits(999), "999");
    }

    fn words() -> AttrWords<'static> {
        AttrWords {
            read_only: "Read-only",
            hidden: "Hidden",
            system: "System",
            archive: "Archive",
            compressed: "Compressed",
            encrypted: "Encrypted",
            offline: "Offline",
            temporary: "Temporary",
            sparse: "Sparse",
        }
    }

    #[test]
    fn attributes_name_the_bits_and_skip_the_rest() {
        let w = words();
        assert_eq!(format_attributes(0, &w), "");
        assert_eq!(format_attributes(FA_READONLY | FA_ARCHIVE, &w), "Read-only, Archive");
        assert_eq!(format_attributes(FA_HIDDEN | FA_SYSTEM | FA_ENCRYPTED, &w), "Hidden, System, Encrypted");
        assert_eq!(format_attributes(0x10 | 0x80, &w), ""); // directory and normal stay quiet
        assert_eq!(format_attributes(FA_OFFLINE | FA_COMPRESSED | FA_TEMPORARY | FA_SPARSE, &w), "Compressed, Offline, Temporary, Sparse");
    }

    #[test]
    fn exif_hides_empty_fields_and_joins_camera_and_gps() {
        let labels = ExifLabels {
            camera: "Camera",
            taken: "Taken",
            exposure: "Exposure",
            aperture: "Aperture",
            iso: "ISO",
            focal: "Focal length",
            gps: "Location",
        };
        let full = ExifIn {
            make: Some("Canon".into()),
            model: Some("EOS R6".into()),
            taken: Some("2026-01-02 03:04".into()),
            exposure: Some("1/200 s".into()),
            fnumber: Some("f/2.8".into()),
            iso: Some("ISO 200".into()),
            focal: Some("50 mm".into()),
            latitude: Some("37.5".into()),
            longitude: Some("127.0".into()),
        };
        let rows = format_exif(&full, &labels);
        assert_eq!(
            rows,
            vec![
                ("Camera".into(), "Canon EOS R6".into()),
                ("Taken".into(), "2026-01-02 03:04".into()),
                ("Exposure".into(), "1/200 s".into()),
                ("Aperture".into(), "f/2.8".into()),
                ("ISO".into(), "ISO 200".into()),
                ("Focal length".into(), "50 mm".into()),
                ("Location".into(), "37.5, 127.0".into()),
            ]
        );
        let sparse = ExifIn { model: Some("  X100  ".into()), latitude: Some("1".into()), make: Some(" ".into()), ..ExifIn::default() };
        assert_eq!(format_exif(&sparse, &labels), vec![("Camera".into(), "X100".into()), ("Location".into(), "1".into())]);
        assert!(format_exif(&ExifIn::default(), &labels).is_empty());
        assert_eq!(exif_line(&rows[..2]), "Camera Canon EOS R6 · Taken 2026-01-02 03:04");
    }

    #[test]
    fn header_wraps_and_ellipsizes_when_narrow() {
        assert_eq!(ellipsize("abcdef", 4), "abc…");
        assert_eq!(ellipsize("ab", 4), "ab");
        let fields = vec!["JPEG image".into(), "1.5 MB".into(), "3840 × 2400".into(), "Modified 2026-10-23".into(), "Read-only, Archive".into()];
        let wide = wrap_fields(&fields, 80);
        assert_eq!(wide.len(), 1);
        let narrow = wrap_fields(&fields, 16);
        assert!(narrow.len() >= 3, "{narrow:?}");
        assert!(narrow.iter().all(|l| l.chars().count() <= 16));
        let lines = header_lines("a-very-long-file-name.txt", &fields, 12);
        assert!(lines[0].ends_with('…'));
        assert!(lines.len() > 2);
        assert_eq!(header_char_budget(160, 16), (160 / 9).max(8) as usize);
    }
}
