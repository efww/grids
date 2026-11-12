use crate::*;
use windows::Win32::Storage::EnhancedStorage::*;
use windows::Win32::Storage::FileSystem::{FileTimeToLocalFileTime, GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
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
    let pic = |flags| shell_pic(path, req.w, req.h, flags);
    if PICS.contains(&e) {
        if let Some(p) = pic(SIIGBF_RESIZETOFIT) {
            return Body::Pic(p);
        }
    }
    if VIDEOS.contains(&e) {
        if let Some(p) = pic(SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY) {
            return Body::Pic(p);
        }
    }
    if AUDIO.contains(&e) {
        return card(path, &[]);
    }
    let heavy = HANDLED.contains(&e) || OFFICE_ZIP.contains(&e) || matches!(e, "pdf" | "eml" | "hwp" | "hwpx" | "hwpt" | "psd" | "psb");
    if heavy && meta.as_ref().is_some_and(|m| m.len() > HEAVY_MAX) {
        let note = (tr("Preview", "미리보기").to_string(), tr("Skipped: larger than 10 MB", "10 MB보다 커서 생략").to_string());
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
        "html" | "htm" | "xhtml" => read_head(path, TEXT_MAX).map(|(b, cut)| Body::Text { text: html_text(&decode_text(&b)), kind: TextKind::Plain, cut }),
        "eml" => read_head(path, 4 << 20).map(|(b, cut)| Body::Text { text: eml_text(&b, KO.load(Ordering::Relaxed)), kind: TextKind::Plain, cut }),
        "url" | "website" => url_card(path),
        "lnk" => Some(lnk_card(path)),
        "hwp" => hwp_pages(path, req).or_else(|| hwp_preview(path, req)),
        "hwpx" | "hwpt" => hwp_pages(path, req).or_else(|| hwpx_preview(path, req)),
        e if OFFICE_ZIP.contains(&e) => office_text(path, e),
        e if ZIPS.contains(&e) => zip_list(path),
        e if PROGRAMS.contains(&e) => Some(card(path, &version_rows(path))),
        "psd" | "psb" => psd_thumb(path, req).map(Body::Pic),
        _ => None,
    };
    if let Some(b) = found {
        return b;
    }
    // unknown type: any thumbnail Windows has for it, text if it reads as text, else the facts
    if let Some(p) = pic(SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY) {
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

pub(crate) unsafe fn bitmap_pix(bmp: HBITMAP) -> Option<Pix> {
    let mut bm = BITMAP::default();
    GetObjectW(bmp.into(), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _));
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    let mut bgra = vec![0u8; (w.max(0) * h * 4) as usize];
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
    Some(Pix { w, h, bgra })
}

/// Picture bytes (PNG, JPEG, GIF, BMP ...) decoded by WIC, shrunk to fit w x h.
pub(crate) unsafe fn decode_pic(bytes: &[u8], w: i32, h: i32) -> Option<Pix> {
    let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
    let stream = SHCreateMemStream(Some(bytes))?;
    let frame = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?.GetFrame(0).ok()?;
    let (mut sw, mut sh) = (0u32, 0u32);
    frame.GetSize(&mut sw, &mut sh).ok()?;
    if sw == 0 || sh == 0 {
        return None;
    }
    let k = (w as f64 / sw as f64).min(h as f64 / sh as f64).min(1.0);
    let (dw, dh) = (((sw as f64 * k) as u32).max(1), ((sh as f64 * k) as u32).max(1));
    let src: IWICBitmapSource = if (dw, dh) != (sw, sh) {
        let sc = wic.CreateBitmapScaler().ok()?;
        sc.Initialize(&frame, dw, dh, WICBitmapInterpolationModeFant).ok()?;
        sc.cast().ok()?
    } else {
        frame.cast().ok()?
    };
    let conv = wic.CreateFormatConverter().ok()?;
    conv.Initialize(&src, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
    let mut bgra = vec![0u8; (dw * dh * 4) as usize];
    conv.CopyPixels(std::ptr::null(), dw * 4, &mut bgra).ok()?;
    Some(Pix { w: dw as i32, h: dh as i32, bgra })
}

// Photoshop keeps a small JPEG (image resource 1036) for file browsers.
unsafe fn psd_thumb(path: &str, req: &PeekReq) -> Option<Pix> {
    let (b, _) = read_head(path, 8 << 20)?;
    let be32 = |at: usize| b.get(at..at + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]) as usize);
    if !b.starts_with(b"8BPS") {
        return None;
    }
    let res = 26 + 4 + be32(26)?; // after the header and the colour mode data
    let (mut p, stop) = (res + 4, (res + 4 + be32(res)?).min(b.len()));
    while p + 12 <= stop && &b[p..p + 4] == b"8BIM" {
        let id = u16::from_be_bytes([b[p + 4], b[p + 5]]);
        let mut q = p + 6 + 1 + b[p + 6] as usize;
        q += q % 2; // the Pascal name is padded to even
        let size = be32(q)?;
        let data = b.get(q + 4..q + 4 + size)?;
        if (id == 1036 || id == 1033) && data.len() > 28 {
            return decode_pic(&data[28..], req.w, req.h);
        }
        p = q + 4 + size + size % 2;
    }
    None
}

// ---------- PDF: Windows' own renderer, pages as pictures ----------

pub(crate) const PDF_MAX: u32 = 50;

// What the paged preview reads from: a PDF, or an HWP/HWPX that rhwp draws one page at a time when asked
// (hwpdoc.rs), so moving on to another file never waits for a whole document.
pub(crate) enum Paged {
    Pdf(PdfDocument),
    Hwp(String),
}

fn pdf_load(path: &str) -> Result<PdfDocument> {
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))?.join()?;
    PdfDocument::LoadFromFileAsync(&file)?.join()
}

pub(crate) fn pdf_open(path: &str) -> Result<Paged> {
    if crate::hwpdoc::is_hwp(path) {
        return Ok(Paged::Hwp(path.to_string()));
    }
    pdf_load(path).map(Paged::Pdf)
}

pub(crate) unsafe fn pdf_page(doc: &Paged, i: u32, w: i32, h: i32) -> Option<Pix> {
    match doc {
        Paged::Pdf(d) => pdf_doc_page(d, i, w, h),
        Paged::Hwp(path) => {
            let one = crate::hwpdoc::page_pdf(path, i)?;
            pdf_doc_page(&pdf_load(&one.to_string_lossy()).ok()?, 0, w, h)
        }
    }
}

/// Page i fitted whole into w x h (px), white paper as the reader would show it.
unsafe fn pdf_doc_page(doc: &PdfDocument, i: u32, w: i32, h: i32) -> Option<Pix> {
    let page = doc.GetPage(i).ok()?;
    let size = page.Size().ok()?;
    let fit = (w as f32).min(h as f32 * size.Width / size.Height.max(1.0));
    let opts = PdfPageRenderOptions::new().ok()?;
    opts.SetDestinationWidth((fit as u32).max(16)).ok()?;
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
    let doc = pdf_load(path).ok()?;
    let total = doc.PageCount().ok()?.min(PDF_MAX);
    let first = pdf_doc_page(&doc, 0, req.w, req.h)?;
    Some(Body::Pages { pages: vec![first], total })
}

// HWP/HWPX drawn page by page (hwpdoc.rs); the embedded preview picture and body text when that fails.
unsafe fn hwp_pages(path: &str, req: &PeekReq) -> Option<Body> {
    let total = crate::hwpdoc::page_count(path)?;
    let first = pdf_page(&Paged::Hwp(path.to_string()), 0, req.w, req.h)?;
    Some(Body::Pages { pages: vec![first], total })
}

// ---------- the header line: icon, type and what matters for this kind of file ----------

unsafe fn prop(ps: &IPropertyStore, k: &PROPERTYKEY) -> Option<String> {
    let v = ps.GetValue(k).ok()?;
    let s = pwstr(PSFormatForDisplayAlloc(k, &v, PDFF_DEFAULT).ok()?);
    (!s.trim().is_empty()).then_some(s)
}

/// (system image list icon, "JPEG 이미지 · 3840 x 2400 · 1.5 MB · 2026-10-23 10:27")
pub(crate) unsafe fn meta(path: &str, body: &Body) -> (i32, Vec<String>) {
    let mut fi = SHFILEINFOW::default();
    let flags = SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_TYPENAME;
    SHGetFileInfoW(&HSTRING::from(path), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut fi), size_of::<SHFILEINFOW>() as u32, flags);
    let mut out = Vec::new();
    let t = String::from_utf16_lossy(&fi.szTypeName[..fi.szTypeName.iter().position(|&c| c == 0).unwrap_or(0)]);
    if !t.is_empty() {
        out.push(t);
    }
    let ko = KO.load(Ordering::Relaxed);
    let e = ext_of(path);
    let ps = SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(&HSTRING::from(path), None, GPS_DEFAULT).ok();
    match body {
        Body::Pic(_) if VIDEOS.contains(&e.as_str()) => {
            if let Some(ps) = &ps {
                let (w, h) = (prop(ps, &PKEY_Video_FrameWidth), prop(ps, &PKEY_Video_FrameHeight));
                if let (Some(w), Some(h)) = (w, h) {
                    out.push(format!("{w} × {h}"));
                }
                out.extend(prop(ps, &PKEY_Media_Duration));
            }
        }
        Body::Pic(_) => out.extend(ps.as_ref().and_then(|ps| prop(ps, &PKEY_Image_Dimensions)).map(|d| d.replace(" x ", " × "))),
        Body::Pages { total, .. } => out.push(if ko { format!("{total}쪽") } else { format!("{total} pages") }),
        Body::Text { text, .. } => out.push(if ko { format!("{}줄", text.lines().count()) } else { format!("{} lines", text.lines().count()) }),
        Body::Table { rows, .. } => out.push(if ko { format!("{}행", rows.len()) } else { format!("{} rows", rows.len()) }),
        Body::List { head, .. } => out.push(head.clone()),
        _ => {}
    }
    if let Ok(m) = std::fs::metadata(path) {
        if m.is_file() {
            out.push(human(m.len()));
        }
        out.extend(when(m.modified()));
    }
    (fi.iIcon, out)
}

// ---------- lists ----------

fn folder_list(path: &str) -> Body {
    let ko = KO.load(Ordering::Relaxed);
    let Ok(rd) = std::fs::read_dir(path) else {
        return Body::List { head: tr("can't open", "열 수 없음").into(), rows: Vec::new() };
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
    let head = if ko {
        format!("폴더 {}개, 파일 {}개, {}", dirs.len(), files.len(), human(total))
    } else {
        format!("{} folders, {} files, {}", dirs.len(), files.len(), human(total))
    };
    let folder = tr("folder", "폴더");
    let rows = dirs.into_iter().map(|d| (d, folder.to_string())).chain(files.into_iter().map(|(n, s)| (n, human(s)))).take(2000).collect();
    Body::List { head, rows }
}

fn zip_list(path: &str) -> Option<Body> {
    let mut f = std::fs::File::open(path).ok()?;
    let es = zip_entries(&mut f)?;
    let files: Vec<&ZipEntry> = es.iter().filter(|e| !e.is_dir()).collect();
    let total: u64 = files.iter().map(|e| e.size).sum();
    let head = if KO.load(Ordering::Relaxed) {
        format!("파일 {}개, 풀면 {}", files.len(), human(total))
    } else {
        format!("{} files, {} unpacked", files.len(), human(total))
    };
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
            let label = tr("Slide", "슬라이드");
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
        Some(Body::Document {
            pic,
            text: out,
            cut,
        })
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
    for mode in [
        STGM_READ | STGM_SHARE_DENY_WRITE,
        STGM_READ | STGM_SHARE_DENY_NONE | STGM_TRANSACTED,
    ] {
        let mut p: *mut std::ffi::c_void = std::ptr::null_mut();
        if StgOpenStorageEx(
            &name,
            mode,
            STGFMT_STORAGE,
            0,
            None,
            None,
            &IStorage::IID,
            &mut p,
        )
        .is_ok()
            && !p.is_null()
        {
            stg = Some(IStorage::from_raw(p));
            break;
        }
    }
    let stg = stg?;
    let read = |storage: &IStorage, s: &str| -> Option<Vec<u8>> {
        let st = storage
            .OpenStream(&HSTRING::from(s), None, STGM_READ | STGM_SHARE_EXCLUSIVE, 0)
            .ok()?;
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut n = 0u32;
            if st
                .Read(buf.as_mut_ptr() as _, buf.len() as u32, Some(&mut n))
                .is_err()
                || n == 0
                || out.len() >= 16 << 20
            {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        (!out.is_empty()).then_some(out)
    };
    let pic = read(&stg, "PrvImage").and_then(|b| decode_pic(&b, req.w, req.h));
    let flags = read(&stg, "FileHeader")
        .filter(|b| b.len() >= 40)
        .map(|b| u32::from_le_bytes(b[36..40].try_into().unwrap()));
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
        if let Ok(body) = stg.OpenStorage(
            &HSTRING::from("BodyText"),
            None,
            STGM_READ | STGM_SHARE_EXCLUSIVE,
            std::ptr::null(),
            0,
        ) {
            for n in 0..1000 {
                if out.len() >= TEXT_MAX {
                    cut = true;
                    break;
                }
                let Some(data) = read(&body, &format!("Section{n}")) else {
                    break;
                };
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
        out = read(&stg, "PrvText")
            .map(|b| utf16(&b, false).trim_end_matches('\0').to_string())
            .unwrap_or_default();
    }
    limit_document(&mut out, &mut cut);
    if pic.is_none() && out.is_empty() {
        None
    } else {
        Some(Body::Document {
            pic,
            text: out,
            cut,
        })
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
    let keys: [(&str, &str, PROPERTYKEY); 8] = [
        ("Title", "제목", PKEY_Title),
        ("Artist", "음악가", PKEY_Music_Artist),
        ("Album", "앨범", PKEY_Music_AlbumTitle),
        ("Length", "길이", PKEY_Media_Duration),
        ("Bit rate", "비트 전송률", PKEY_Audio_EncodingBitrate),
        ("Dimensions", "크기(픽셀)", PKEY_Image_Dimensions),
        ("Author", "만든 이", PKEY_Author),
        ("Pages", "페이지", PKEY_Document_PageCount),
    ];
    let mut out = Vec::new();
    for (en, ko, k) in keys {
        let Ok(v) = ps.GetValue(&k) else { continue };
        if let Ok(s) = PSFormatForDisplayAlloc(&k, &v, PDFF_DEFAULT) {
            let s = pwstr(s);
            if !s.trim().is_empty() {
                out.push((tr(en, ko).to_string(), s));
            }
        }
    }
    out
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
        rows.push((tr("Size", "크기").into(), format!("{} B", group_digits(m.len()))));
        if let Some(t) = when(m.created()) {
            rows.push((tr("Created", "만든 날짜").into(), t));
        }
    }
    if let Some(dir) = Path::new(path).parent() {
        rows.push((tr("Folder", "위치").into(), dir.display().to_string()));
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
                rows.push((tr("Target", "대상").into(), s(&buf)));
            }
            buf[0] = 0;
            if link.GetArguments(&mut buf).is_ok() && buf[0] != 0 {
                rows.push((tr("Arguments", "인수").into(), s(&buf)));
            }
            buf[0] = 0;
            if link.GetWorkingDirectory(&mut buf).is_ok() && buf[0] != 0 {
                rows.push((tr("Start in", "시작 위치").into(), s(&buf)));
            }
            buf[0] = 0;
            if link.GetDescription(&mut buf).is_ok() && buf[0] != 0 {
                rows.push((tr("Comment", "설명").into(), s(&buf)));
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
    for (key, en, ko) in [
        ("FileDescription", "Description", "설명"),
        ("ProductName", "Product", "제품"),
        ("CompanyName", "Company", "회사"),
        ("FileVersion", "Version", "버전"),
    ] {
        if let Some((p, len)) = query(&format!("\\StringFileInfo\\{lang}\\{key}")).filter(|(_, l)| *l > 1) {
            let v = String::from_utf16_lossy(std::slice::from_raw_parts(p as *const u16, len as usize - 1));
            if !v.trim().is_empty() {
                rows.push((tr(en, ko).to_string(), v.trim().to_string()));
            }
        }
    }
    rows
}
