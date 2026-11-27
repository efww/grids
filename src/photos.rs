use crate::*;
use std::os::windows::fs::MetadataExt;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::{PROPBAG2, PROPVARIANT};

// ---------- ⇄ menu: smaller copies of the selected photos ----------
// Each photo in the selection is read with WIC, shrunk so its long side is 1280 / 1920 / 2560 px, turned upright
// (a JPEG's EXIF orientation) and saved next to it as "Name (1920px).jpg": a PNG stays PNG, a picture with see-through
// pixels becomes PNG, everything else JPEG. Only the date taken and the colour profile go along (no GPS or camera
// data); the copy gets the original's date modified. A file is never replaced; the originals and the selection stay.
// Like macOS Finder's Convert Image quick action, PowerToys Image Resizer, Directory Opus' image convert.

pub(crate) const ID_PHOTO: usize = 1155; // ⇄ menu: + 0 small, + 1 medium, + 2 large (1154 is ID_EXPLORER)
pub(crate) const PHOTO_EDGES: [u32; 3] = [1280, 1920, 2560];
const PIXEL_MAX: u64 = 250_000_000; // bigger pictures count as unreadable (a 200 MP phone photo still goes)
const JPEG_QUALITY: f32 = 0.85;
const TAKEN_JPEG: &str = "/app1/ifd/exif/{ushort=36867}"; // EXIF DateTimeOriginal
// decoded pixel formats whose colour profile doesn't fit the RGB copy (CMYK and greyscale originals)
const NOT_RGB: [GUID; 13] = [
    GUID_WICPixelFormat32bppCMYK,
    GUID_WICPixelFormat64bppCMYK,
    GUID_WICPixelFormat40bppCMYKAlpha,
    GUID_WICPixelFormat80bppCMYKAlpha,
    GUID_WICPixelFormatBlackWhite,
    GUID_WICPixelFormat2bppGray,
    GUID_WICPixelFormat4bppGray,
    GUID_WICPixelFormat8bppGray,
    GUID_WICPixelFormat16bppGray,
    GUID_WICPixelFormat16bppGrayFixedPoint,
    GUID_WICPixelFormat16bppGrayHalf,
    GUID_WICPixelFormat32bppGrayFloat,
    GUID_WICPixelFormat32bppGrayFixedPoint,
];

pub(crate) fn is_photo_pick(p: usize) -> bool {
    (ID_PHOTO..ID_PHOTO + PHOTO_EDGES.len()).contains(&p)
}

// a run is under way: the menu entry is grey meanwhile
static SHRINKING: AtomicBool = AtomicBool::new(false);
// Grids is closing, or Windows asks to restart / sign out: the worker stops before its next photo
// (the window waits for JOBS; a restart is let through once the photo being made is done). Cleared by the next run.
pub(crate) static PHOTOS_STOP: AtomicBool = AtomicBool::new(false);

struct ShrinkingDone;
impl Drop for ShrinkingDone {
    fn drop(&mut self) {
        SHRINKING.store(false, Ordering::SeqCst);
    }
}

// The pictures this works on: the date-folder photo list (raw files mostly can't be read: "couldn't read") and BMP.
pub(crate) fn shrink_ext(path: &str) -> bool {
    let name = path.rsplit('\\').next().unwrap_or(path);
    photo_ext(path) || name.rsplit_once('.').is_some_and(|(_, e)| e.eq_ignore_ascii_case("bmp"))
}

// The new size with the long side at `edge`, the short one in proportion (at least 1 px); None when it is no bigger.
pub(crate) fn fit_size(w: u32, h: u32, edge: u32) -> Option<(u32, u32)> {
    if w == 0 || h == 0 || w.max(h) <= edge {
        return None;
    }
    let short = |s: u32, l: u32| ((s as u64 * edge as u64 + l as u64 / 2) / l as u64).max(1) as u32;
    Some(if w >= h { (edge, short(h, w)) } else { (short(w, h), edge) })
}

// Pixels of 4 bytes, w × h, rows top down, turned upright for EXIF orientation `o` (1 or unknown: as they are).
// Returns the new width and height (swapped for 5–8) and the pixels.
pub(crate) fn upright(w: usize, h: usize, px: Vec<u8>, o: u16) -> (usize, usize, Vec<u8>) {
    if !(2..=8).contains(&o) || px.len() != w * h * 4 {
        return (w, h, px);
    }
    let nw = if o >= 5 { h } else { w };
    let mut out = vec![0u8; px.len()];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match o {
                2 => (w - 1 - x, y),         // mirrored
                3 => (w - 1 - x, h - 1 - y), // upside down
                4 => (x, h - 1 - y),         // mirrored upside down
                5 => (y, x),                 // transposed
                6 => (h - 1 - y, x),         // turn 90° clockwise
                7 => (h - 1 - y, w - 1 - x), // transversed
                _ => (y, w - 1 - x),         // 8: turn 90° counter-clockwise
            };
            let (s, d) = ((y * w + x) * 4, (ny * nw + nx) * 4);
            out[d..d + 4].copy_from_slice(&px[s..s + 4]);
        }
    }
    if o >= 5 { (h, w, out) } else { (w, h, out) }
}

// "IMG_1234 (1920px).jpg", " (k)" from the second try on. A JPEG keeps its own extension, as does a PNG made PNG;
// other pictures get ".jpg", or ".png" when they keep see-through pixels.
pub(crate) fn shrunk_name(file: &str, edge: u32, png: bool, k: u32) -> String {
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, e),
        _ => (file, ""),
    };
    let same: &[&str] = if png { &["png"] } else { &["jpg", "jpeg", "jpe"] };
    let ext = match same.iter().any(|x| x.eq_ignore_ascii_case(ext)) {
        true => ext,
        false if png => "png",
        false => "jpg",
    };
    let n = if k >= 2 { format!(" ({k})") } else { String::new() };
    format!("{stem} ({edge}px){n}.{ext}")
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct ShrinkCount {
    pub(crate) made: usize,
    pub(crate) small: usize,  // already no bigger than the size asked for
    pub(crate) online: usize, // online-only cloud files: never downloaded for this
    pub(crate) bad: usize,    // couldn't be read (no codec, raw, damaged, too big) or encoded
    pub(crate) unsaved: usize, // read and shrunk, but the copy couldn't be written (read-only folder, disk full, path too long)
}

// One photo's end, short of a failure to read it.
enum Shrunk {
    Made,
    Small,   // no bigger than the size asked for already
    Unsaved, // the copy couldn't be written next to it
}

// The status line once the run is over.
pub(crate) fn done_text(ko: bool, c: &ShrinkCount) -> String {
    let mut t = match (ko, c.made) {
        (true, 0) => "줄인 사본을 만들지 않았습니다".to_string(),
        (true, n) => format!("사진 {n}장의 줄인 사본을 만들었습니다"),
        (false, 0) => "No smaller copies made".to_string(),
        (false, 1) => "Made a smaller copy of 1 photo".to_string(),
        (false, n) => format!("Made smaller copies of {n} photos"),
    };
    let parts = [
        (c.small, "already small enough", "이미 작아서 건너뜀"),
        (c.online, "online-only, not downloaded", "온라인 전용이라 건너뜀"),
        (c.bad, "couldn't be read", "못 읽음"),
        (c.unsaved, "couldn't be saved", "저장하지 못함"),
    ];
    for (n, en, k) in parts.into_iter().filter(|p| p.0 > 0) {
        t.push_str(&if ko { format!(" · {k} {n}") } else { format!(" · {n} {en}") });
    }
    t
}

// "Make smaller copies of the selected photos ▸ Small — long side 1280 px / Medium — 1920 px / Large — 2560 px"
pub(crate) unsafe fn photo_menu(menu: HMENU, ok: bool) {
    let Ok(sub) = CreatePopupMenu() else { return };
    let hint = tr("Copies go next to the originals as “Name (1920px).jpg”", "원본 옆에 ‘이름 (1920px).jpg’로 저장, 원본은 그대로");
    _ = AppendMenuW(sub, MF_STRING | MF_GRAYED, 0, &HSTRING::from(hint));
    _ = AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null());
    let names = [tr("&Small — long side 1280 px", "작게 — 긴 변 1280px(&S)"), tr("&Medium — 1920 px", "중간 — 1920px(&M)"), tr("&Large — 2560 px", "크게 — 2560px(&L)")];
    for (k, name) in names.into_iter().enumerate() {
        _ = AppendMenuW(sub, MF_STRING, ID_PHOTO + k, &HSTRING::from(name));
    }
    let busy = SHRINKING.load(Ordering::SeqCst);
    let flags = if ok && !busy { MF_POPUP } else { MF_POPUP | MF_GRAYED };
    _ = AppendMenuW(menu, flags, sub.0 as usize, &HSTRING::from(tr2(M_PHOTO)));
}

// Live for files only, all in one file system folder, at least one of them a picture.
pub(crate) fn photo_ok(picked: &std::result::Result<Vec<String>, Pick>, no_dirs: bool, here: &str) -> bool {
    org_ok(picked, no_dirs, here) && picked.as_ref().is_ok_and(|p| p.iter().any(|f| shrink_ext(f)))
}

// Picked from the ⇄ menu: the selection it counted, checked again; the pictures are made on a worker thread.
pub(crate) unsafe fn shrink_photos(src: usize, k: usize, picked: std::result::Result<Vec<String>, Pick>) {
    let here = app(|a| a.s.paths[src].clone());
    let (Some(&edge), true) = (PHOTO_EDGES.get(k), photo_ok(&picked, true, &here)) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let photos: Vec<String> = picked.unwrap_or_default().into_iter().filter(|p| shrink_ext(p)).collect();
    if SHRINKING.swap(true, Ordering::SeqCst) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    PHOTOS_STOP.store(false, Ordering::Relaxed); // a cancelled restart may have left it up
    let done = ShrinkingDone; // released when the worker ends, or right away if it can't start
    let ko = KO.load(Ordering::Relaxed);
    set_note(progress_text(ko, 0, photos.len()));
    let busy = Busy::new();
    let job = std::thread::Builder::new().spawn(move || {
        let _busy = busy;
        let _done = done;
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let c = shrink_all(&photos, edge, ko);
        *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(done_text(ko, &c));
        if com {
            CoUninitialize();
        }
    });
    match job {
        Ok(job) => keep_job(job),
        Err(_) => _ = MessageBeep(MB_ICONWARNING),
    }
}

fn progress_text(ko: bool, k: usize, n: usize) -> String {
    if ko { format!("사진 줄이는 중 {k}/{n}…") } else { format!("Shrinking photos {k}/{n}…") }
}

// Worker thread: one photo after the other; a failure only counts against that photo.
unsafe fn shrink_all(photos: &[String], edge: u32, ko: bool) -> ShrinkCount {
    let mut c = ShrinkCount::default();
    let Ok(wic) = CoCreateInstance::<_, IWICImagingFactory>(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) else {
        c.bad = photos.len();
        return c;
    };
    let mut shown = Instant::now();
    for (k, p) in photos.iter().enumerate() {
        if PHOTOS_STOP.load(Ordering::Relaxed) {
            break;
        }
        // the status line at most once a second: each new note is read out by screen readers
        if k > 0 && shown.elapsed() >= Duration::from_secs(1) {
            *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(progress_text(ko, k, photos.len()));
            shown = Instant::now();
        }
        let Ok(m) = std::fs::metadata(p) else {
            c.bad += 1; // gone since the menu, or its folder was
            continue;
        };
        if cloud_only(m.file_attributes()) {
            c.online += 1;
            continue;
        }
        match m.is_file().then(|| shrink_file(&wic, p, edge, m.modified().ok())) {
            Some(Ok(Shrunk::Made)) => c.made += 1,
            Some(Ok(Shrunk::Small)) => c.small += 1,
            Some(Ok(Shrunk::Unsaved)) => c.unsaved += 1,
            _ => c.bad += 1,
        }
    }
    c
}

// One photo: an Err is a picture that couldn't be read (or encoded).
unsafe fn shrink_file(wic: &IWICImagingFactory, path: &str, edge: u32, modified: Option<std::time::SystemTime>) -> Result<Shrunk> {
    let dec = wic.CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
    let kind = dec.GetContainerFormat()?;
    let (jpeg, png_in) = (kind == GUID_ContainerFormatJpeg, kind == GUID_ContainerFormatPng);
    let frame = dec.GetFrame(0)?;
    let (mut w, mut h) = (0u32, 0u32);
    frame.GetSize(&mut w, &mut h)?;
    if w as u64 * h as u64 > PIXEL_MAX {
        return Err(E_OUTOFMEMORY.into());
    }
    let Some((dw, dh)) = fit_size(w, h, edge) else { return Ok(Shrunk::Small) };
    let reader = frame.GetMetadataQueryReader().ok();
    let tag = |name: &str| {
        let mut v = PROPVARIANT::default();
        reader.as_ref()?.GetMetadataByName(&HSTRING::from(name), &mut v).ok().map(|_| v)
    };
    // only a JPEG's tag is applied: other decoders (HEIF) may turn the picture upright themselves
    let orient = if jpeg { tag("/app1/ifd/{ushort=274}").and_then(|v| u16::try_from(&v).ok()).unwrap_or(1) } else { 1 };
    let taken = tag(if jpeg { TAKEN_JPEG } else { "/ifd/exif/{ushort=36867}" });
    // the colour profile (a phone's Display P3): without it the copy's colours look washed out
    // how many: asked the documented way, with no array (the wrapper would pass a dangling one)
    // only from an RGB-family original: the copy's pixels are RGB, a CMYK or grey profile would skew its colours
    let mut n = 0u32;
    let mut ctx: Vec<Option<IWICColorContext>> = Vec::new();
    let rgb = frame.GetPixelFormat().is_ok_and(|f| !NOT_RGB.contains(&f));
    let count = (Interface::vtable(&frame).GetColorContexts)(Interface::as_raw(&frame), 0, std::ptr::null_mut(), &mut n);
    if rgb && count.is_ok() && (1..=8).contains(&n) {
        ctx = (0..n).map(|_| wic.CreateColorContext().ok()).collect();
        let got = ctx.iter().all(Option::is_some) && frame.GetColorContexts(&mut ctx, &mut n).is_ok();
        ctx.retain(|c| got && c.as_ref().and_then(|c| c.GetType().ok()) == Some(WICColorContextProfile));
    }
    // shrunk straight from the decoder (premultiplied, so see-through edges don't fringe), then held in memory
    let pre = wic.CreateFormatConverter()?;
    pre.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
    let sc = wic.CreateBitmapScaler()?;
    sc.Initialize(&pre, dw, dh, WICBitmapInterpolationModeFant)?;
    let straight = wic.CreateFormatConverter()?;
    straight.Initialize(&sc, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
    let mut px = vec![0u8; dw as usize * dh as usize * 4];
    straight.CopyPixels(std::ptr::null(), dw * 4, &mut px)?;
    let see_through = px.chunks_exact(4).any(|p| p[3] != 255);
    let (ow, oh, px) = upright(dw as usize, dh as usize, px, orient);
    let (ow, oh) = (ow as u32, oh as u32);
    let bmp = wic.CreateBitmapFromMemory(ow, oh, &GUID_WICPixelFormat32bppBGRA, ow * 4, &px)?;
    let png = png_in || see_through;
    let fmt = if see_through { GUID_WICPixelFormat32bppBGRA } else { GUID_WICPixelFormat24bppBGR };
    let out_px = wic.CreateFormatConverter()?;
    out_px.Initialize(&bmp, &fmt, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
    // encoded in memory, then written under a free name (save_unique: never over a file, nothing half written left)
    let out = SHCreateMemStream(None).ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
    let enc = wic.CreateEncoder(if png { &GUID_ContainerFormatPng } else { &GUID_ContainerFormatJpeg }, std::ptr::null())?;
    enc.Initialize(&out, WICBitmapEncoderNoCache)?;
    let (mut fr, mut props) = (None, None);
    enc.CreateNewFrame(&mut fr, &mut props)?;
    let fr = fr.ok_or_else(|| Error::from(E_FAIL))?;
    if let (false, Some(bag)) = (png, props.as_ref()) {
        let name = HSTRING::from("ImageQuality");
        let opt = PROPBAG2 { pstrName: PWSTR(name.as_ptr() as *mut u16), ..Default::default() };
        _ = bag.Write(1, &opt, &VARIANT::from(JPEG_QUALITY));
    }
    fr.Initialize(props.as_ref())?;
    fr.SetSize(ow, oh)?;
    if let Some(first) = ctx.first() {
        _ = fr.SetColorContexts(std::slice::from_ref(first));
    }
    // the date taken, as it was read; no orientation tag: the pixels are upright now
    if let (false, Some(t)) = (png, &taken) {
        if let Ok(wr) = fr.GetMetadataQueryWriter() {
            _ = wr.SetMetadataByName(&HSTRING::from(TAKEN_JPEG), t);
        }
    }
    fr.WriteSource(&out_px, std::ptr::null())?;
    fr.Commit()?;
    enc.Commit()?;
    let mut len = 0u64;
    out.Seek(0, STREAM_SEEK_END, Some(&mut len))?;
    out.Seek(0, STREAM_SEEK_SET, None)?;
    let mut bytes = vec![0u8; len as usize];
    let mut read = 0u32;
    out.Read(bytes.as_mut_ptr().cast(), len as u32, Some(&mut read)).ok()?;
    bytes.truncate(read as usize);
    let file = Path::new(path);
    let (Some(dir), Some(fname)) = (file.parent(), file.file_name()) else { return Err(E_FAIL.into()) };
    let fname = fname.to_string_lossy();
    let Ok(saved) = save_unique(dir, |k| shrunk_name(&fname, edge, png, k), &bytes) else { return Ok(Shrunk::Unsaved) };
    if let (Some(t), Ok(f)) = (modified, std::fs::File::options().write(true).open(dir.join(saved))) {
        _ = f.set_modified(t);
    }
    Ok(Shrunk::Made)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_size_bounds() {
        assert_eq!(fit_size(4000, 3000, 1920), Some((1920, 1440)));
        assert_eq!(fit_size(3000, 4000, 1920), Some((1440, 1920))); // portrait: the height is the long side
        assert_eq!(fit_size(1920, 1080, 1920), None); // long side equal: nothing to do
        assert_eq!(fit_size(1921, 1081, 1920), Some((1920, 1080)));
        assert_eq!(fit_size(1000, 800, 1280), None);
        assert_eq!(fit_size(100_000, 1, 1280), Some((1280, 1))); // a 1 px strip stays 1 px
        assert_eq!(fit_size(1, 100_000, 2560), Some((1, 2560)));
        assert_eq!(fit_size(0, 5000, 1280), None);
        assert_eq!(fit_size(5000, 5000, 2560), Some((2560, 2560)));
    }

    // a 3 × 2 picture whose pixels are numbered 0..6 (row by row), through each orientation
    #[test]
    fn upright_all_orientations() {
        let src: Vec<u8> = (0..6u8).flat_map(|v| [v, 0, 0, 255]).collect();
        let got = |o: u16| {
            let (w, h, px) = upright(3, 2, src.clone(), o);
            (w, h, px.chunks_exact(4).map(|p| p[0]).collect::<Vec<u8>>())
        };
        // stored:  0 1 2
        //          3 4 5
        assert_eq!(got(1), (3, 2, vec![0, 1, 2, 3, 4, 5]));
        assert_eq!(got(0), (3, 2, vec![0, 1, 2, 3, 4, 5])); // no tag / unknown: as it is
        assert_eq!(got(9), (3, 2, vec![0, 1, 2, 3, 4, 5]));
        assert_eq!(got(2), (3, 2, vec![2, 1, 0, 5, 4, 3]));
        assert_eq!(got(3), (3, 2, vec![5, 4, 3, 2, 1, 0]));
        assert_eq!(got(4), (3, 2, vec![3, 4, 5, 0, 1, 2]));
        assert_eq!(got(5), (2, 3, vec![0, 3, 1, 4, 2, 5]));
        assert_eq!(got(6), (2, 3, vec![3, 0, 4, 1, 5, 2])); // the stored first row becomes the right column
        assert_eq!(got(7), (2, 3, vec![5, 2, 4, 1, 3, 0]));
        assert_eq!(got(8), (2, 3, vec![2, 5, 1, 4, 0, 3])); // the stored first row becomes the left column
    }

    #[test]
    fn shrunk_names() {
        assert_eq!(shrunk_name("IMG_1234.JPG", 1920, false, 1), "IMG_1234 (1920px).JPG");
        assert_eq!(shrunk_name("IMG_1234.jpeg", 1280, false, 2), "IMG_1234 (1280px) (2).jpeg");
        assert_eq!(shrunk_name("photo.heic", 2560, false, 1), "photo (2560px).jpg");
        assert_eq!(shrunk_name("scan.tiff", 1920, false, 3), "scan (1920px) (3).jpg");
        assert_eq!(shrunk_name("logo.png", 1280, true, 1), "logo (1280px).png");
        assert_eq!(shrunk_name("sticker.webp", 1280, true, 1), "sticker (1280px).png");
        assert_eq!(shrunk_name("a.b.c.jpg", 1920, false, 1), "a.b.c (1920px).jpg");
        assert_eq!(shrunk_name(".jpg", 1920, false, 1), ".jpg (1920px).jpg");
    }

    #[test]
    fn photo_files_and_picks() {
        assert!(shrink_ext(r"C:\p\IMG_1.JPG") && shrink_ext(r"C:\p\a.heic") && shrink_ext(r"C:\p\scan.bmp") && shrink_ext(r"C:\p\x.png"));
        assert!(!shrink_ext(r"C:\p\notes.txt") && !shrink_ext(r"C:\p.bmp\notes") && !shrink_ext(r"C:\p\bmp"));
        assert!(is_photo_pick(ID_PHOTO) && is_photo_pick(ID_PHOTO + 2) && !is_photo_pick(ID_PHOTO + 3) && !is_photo_pick(ID_PHOTO - 1));
        assert!(is_shell_pick(ID_PHOTO + 1) && open_in_pick(ID_PHOTO) == OpenPick::Shell);
    }

    #[test]
    fn done_texts() {
        let c = ShrinkCount { made: 12, small: 2, online: 0, bad: 1, unsaved: 0 };
        assert_eq!(done_text(false, &c), "Made smaller copies of 12 photos · 2 already small enough · 1 couldn't be read");
        assert_eq!(done_text(true, &c), "사진 12장의 줄인 사본을 만들었습니다 · 이미 작아서 건너뜀 2 · 못 읽음 1");
        let full = ShrinkCount { made: 3, unsaved: 2, ..Default::default() };
        assert_eq!(done_text(false, &full), "Made smaller copies of 3 photos · 2 couldn't be saved");
        assert_eq!(done_text(true, &full), "사진 3장의 줄인 사본을 만들었습니다 · 저장하지 못함 2");
        let one = ShrinkCount { made: 1, ..Default::default() };
        assert_eq!(done_text(false, &one), "Made a smaller copy of 1 photo");
        let none = ShrinkCount { small: 3, online: 1, ..Default::default() };
        assert_eq!(done_text(false, &none), "No smaller copies made · 3 already small enough · 1 online-only, not downloaded");
        assert_eq!(done_text(true, &none), "줄인 사본을 만들지 않았습니다 · 이미 작아서 건너뜀 3 · 온라인 전용이라 건너뜀 1");
    }

    // Encodes a w × h picture (top-left quarter red, the rest blue) with WIC, with an orientation and a date taken
    // for a JPEG.
    unsafe fn make_pic(wic: &IWICImagingFactory, path: &Path, w: u32, h: u32, jpeg: bool, orient: u16, taken: &str) {
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let red = x < w / 2 && y < h / 2;
                px[i..i + 4].copy_from_slice(&if red { [0, 0, 255, 255] } else { [255, 0, 0, 255] });
            }
        }
        let bmp = wic.CreateBitmapFromMemory(w, h, &GUID_WICPixelFormat32bppBGRA, w * 4, &px).unwrap();
        let conv = wic.CreateFormatConverter().unwrap();
        conv.Initialize(&bmp, &GUID_WICPixelFormat24bppBGR, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).unwrap();
        let stream = wic.CreateStream().unwrap();
        stream.InitializeFromFilename(&HSTRING::from(path.to_string_lossy().as_ref()), GENERIC_WRITE.0).unwrap();
        let enc = wic.CreateEncoder(if jpeg { &GUID_ContainerFormatJpeg } else { &GUID_ContainerFormatPng }, std::ptr::null()).unwrap();
        enc.Initialize(&stream, WICBitmapEncoderNoCache).unwrap();
        let (mut fr, mut props) = (None, None);
        enc.CreateNewFrame(&mut fr, &mut props).unwrap();
        let fr = fr.unwrap();
        fr.Initialize(props.as_ref()).unwrap();
        if jpeg {
            let wr = fr.GetMetadataQueryWriter().unwrap();
            wr.SetMetadataByName(w!("/app1/ifd/{ushort=274}"), &PROPVARIANT::from(orient)).unwrap();
            let t = PROPVARIANT::from(BSTR::from(taken));
            // EXIF wants an ANSI string: let the property system turn the BSTR into one
            let mut a = PROPVARIANT::default();
            _ = windows::Win32::System::Com::StructuredStorage::PropVariantChangeType(&mut a, &t, windows::Win32::System::Com::StructuredStorage::PROPVAR_CHANGE_FLAGS(0), windows::Win32::System::Variant::VT_LPSTR);
            wr.SetMetadataByName(&HSTRING::from(TAKEN_JPEG), &a).unwrap();
        }
        fr.WriteSource(&conv, std::ptr::null()).unwrap();
        fr.Commit().unwrap();
        enc.Commit().unwrap();
    }

    // (width, height, red at the top-left, red at the top-right, the date taken) of a picture file
    unsafe fn look(wic: &IWICImagingFactory, path: &Path) -> (u32, u32, bool, bool, Option<String>) {
        let dec = wic.CreateDecoderFromFilename(&HSTRING::from(path.to_string_lossy().as_ref()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).unwrap();
        let frame = dec.GetFrame(0).unwrap();
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h).unwrap();
        let conv = wic.CreateFormatConverter().unwrap();
        conv.Initialize(&frame, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).unwrap();
        let mut px = vec![0u8; (w * h * 4) as usize];
        conv.CopyPixels(std::ptr::null(), w * 4, &mut px).unwrap();
        let red = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            px[i + 2] > 200 && px[i] < 60
        };
        let mut v = PROPVARIANT::default();
        let taken = frame.GetMetadataQueryReader().ok().and_then(|r| r.GetMetadataByName(&HSTRING::from(TAKEN_JPEG), &mut v).ok()).map(|_| v.to_string());
        (w, h, red(w / 8, h / 8), red(w - 1 - w / 8, h / 8), taken)
    }

    #[test]
    fn shrinks_real_pictures() {
        unsafe {
            _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let d = std::env::temp_dir().join(format!("grids-photos-{}", std::process::id()));
            _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            // a 4000 × 3000 PNG: 1920 × 1440, still PNG, the original as it was, the same date modified
            let big = d.join("big.png");
            make_pic(&wic, &big, 4000, 3000, false, 1, "");
            let before = std::fs::read(&big).unwrap();
            let t = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
            std::fs::File::options().write(true).open(&big).unwrap().set_modified(t).unwrap();
            assert!(matches!(shrink_file(&wic, &big.to_string_lossy(), 1920, Some(t)), Ok(Shrunk::Made)));
            let out = d.join("big (1920px).png");
            let (w, h, tl, tr_, _) = look(&wic, &out);
            assert_eq!((w, h, tl, tr_), (1920, 1440, true, false));
            assert_eq!(std::fs::metadata(&out).unwrap().modified().unwrap(), t);
            assert_eq!(std::fs::read(&big).unwrap(), before);
            // again: a second name, the first copy left alone
            let first = std::fs::read(&out).unwrap();
            assert!(matches!(shrink_file(&wic, &big.to_string_lossy(), 1920, None), Ok(Shrunk::Made)));
            assert!(d.join("big (1920px) (2).png").exists());
            assert_eq!(std::fs::read(&out).unwrap(), first);
            // already small: nothing made
            assert!(matches!(shrink_file(&wic, &big.to_string_lossy(), 4000, None), Ok(Shrunk::Small)));
            // a JPEG stored on its side (orientation 6: turn 90° clockwise): upright, the red corner top right
            let side = d.join("side.jpg");
            make_pic(&wic, &side, 1600, 1200, true, 6, "2024:05:01 12:34:56");
            assert!(matches!(shrink_file(&wic, &side.to_string_lossy(), 1280, None), Ok(Shrunk::Made)));
            let (w, h, tl, tr_, taken) = look(&wic, &d.join("side (1280px).jpg"));
            assert_eq!((w, h, tl, tr_), (960, 1280, false, true));
            assert_eq!(taken.as_deref(), Some("2024:05:01 12:34:56"));
            // a file that isn't a picture: an error, nothing written
            let txt = d.join("fake.jpg");
            std::fs::write(&txt, b"not a picture").unwrap();
            assert!(shrink_file(&wic, &txt.to_string_lossy(), 1280, None).is_err());
            assert!(!d.join("fake (1280px).jpg").exists());
            _ = std::fs::remove_dir_all(&d);
        }
    }
}
