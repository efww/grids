// Embeds app.manifest (common controls v6 + per-monitor DPI), app.ico (light background, group 1),
// and assets/maguro-dark-bg.ico (dark background, group 2).
// Writes a binary .res by hand and lets windres only convert it to COFF, so no C preprocessor is needed.
fn main() {
    emit_git_hash();
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=app.ico");
    println!("cargo:rerun-if-changed=assets/maguro-dark-bg.ico");
    println!("cargo:rerun-if-changed=build.rs");
    let out = std::env::var("OUT_DIR").unwrap();
    let entry = |size: usize, typ: u16, id: u16, lang: u16| {
        let mut h = Vec::new();
        h.extend((size as u32).to_le_bytes());
        h.extend(32u32.to_le_bytes()); // header size
        h.extend([0xFFFF, typ, 0xFFFF, id].iter().flat_map(|w: &u16| w.to_le_bytes()));
        h.extend(0u32.to_le_bytes()); // data version
        h.extend([0u16, lang].iter().flat_map(|w| w.to_le_bytes())); // memory flags, language
        h.extend([0u32; 2].iter().flat_map(|d| d.to_le_bytes())); // version, characteristics
        h
    };
    let mut res = entry(0, 0, 0, 0); // mandatory empty first entry
    let mut add = |typ: u16, id: u16, data: &[u8]| {
        res.extend(entry(data.len(), typ, id, 0x0409));
        res.extend(data);
        res.resize(res.len().next_multiple_of(4), 0);
    };
    let v = file_version();
    let manifest = std::fs::read_to_string("app.manifest").unwrap().replace("@VERSION@", &format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3]));
    add(24, 1, manifest.as_bytes()); // RT_MANIFEST, assembly version follows Cargo.toml

    // Group 1: light-bg app.ico (exe and window class). Group 2: dark-bg logo for the themed title bar and About.
    let next = embed_group(&mut add, &std::fs::read("app.ico").unwrap(), 1, 1);
    embed_group(&mut add, &std::fs::read("assets/maguro-dark-bg.ico").unwrap(), 2, next);
    add(16, 1, &version_info()); // RT_VERSION
    std::fs::write(format!("{out}/app.res"), res).unwrap();

    let obj = format!("{out}/app.res.o");
    let windres = std::env::var("WINDRES").unwrap_or("windres".into());
    let ok = std::process::Command::new(windres)
        .args(["-J", "res", "-i", &format!("{out}/app.res"), "-O", "coff", "-o", &obj])
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "windres failed: set WINDRES to msys2's windres.exe");
    println!("cargo:rustc-link-arg-bins={obj}");
}

/// One RT_ICON per image, then an RT_GROUP_ICON. Returns the next free RT_ICON id.
fn embed_group(add: &mut impl FnMut(u16, u16, &[u8]), ico: &[u8], group_id: u16, mut icon_id: u16) -> u16 {
    let count = u16::from_le_bytes([ico[4], ico[5]]);
    let mut group = vec![0u8, 0, 1, 0];
    group.extend(count.to_le_bytes());
    for k in 0..count as usize {
        let e = &ico[6 + 16 * k..22 + 16 * k];
        let size = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
        add(3, icon_id, &ico[offset..offset + size]); // RT_ICON
        group.extend(&e[..12]); // width, height, colours, reserved, planes, bit count, bytes
        group.extend(icon_id.to_le_bytes());
        icon_id += 1;
    }
    add(14, group_id, &group); // RT_GROUP_ICON
    icon_id
}

// VS_VERSIONINFO. wLength is the node size in bytes and includes the zero bytes
// that keep every node DWORD-aligned. String wValueLength counts UTF-16 code
// units, including the NUL; binary nodes (fixed info, Translation) count bytes.
// wType is 1 for text and 0 for binary.
fn utf16z(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity((s.len() + 1) * 2);
    for u in s.encode_utf16() {
        v.extend(u.to_le_bytes());
    }
    v.extend(0u16.to_le_bytes());
    v
}

fn version_node(key: &str, value: &[u8], value_len: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut body = utf16z(key);
    while (6 + body.len()) % 4 != 0 {
        body.push(0);
    }
    body.extend_from_slice(value);
    while (6 + body.len()) % 4 != 0 {
        body.push(0);
    }
    for child in children {
        body.extend_from_slice(child);
    }
    while (6 + body.len()) % 4 != 0 {
        body.push(0);
    }
    let w_length = (6 + body.len()) as u16;
    let mut out = Vec::with_capacity(w_length as usize);
    out.extend(w_length.to_le_bytes());
    out.extend(value_len.to_le_bytes());
    out.extend(u16::from(text).to_le_bytes());
    out.extend(body);
    assert_eq!(out.len(), w_length as usize);
    assert_eq!(out.len() % 4, 0);
    out
}

fn version_string(key: &str, value: &str) -> Vec<u8> {
    let words: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: Vec<u8> = words.iter().copied().flat_map(|u| u.to_le_bytes()).collect();
    version_node(key, &bytes, words.len() as u16, true, &[])
}

fn file_version() -> [u16; 4] {
    let mut v = [0u16; 4];
    for (i, part) in env!("CARGO_PKG_VERSION").split(['.', '-', '+']).take(4).enumerate() {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse() {
            v[i] = n;
        }
    }
    v
}

fn version_info() -> Vec<u8> {
    let ver = file_version();
    let mut fixed = Vec::with_capacity(52);
    let dw = |b: &mut Vec<u8>, n: u32| b.extend(n.to_le_bytes());
    dw(&mut fixed, 0xFEEF_04BD);
    dw(&mut fixed, 0x0001_0000);
    let ms = ((ver[0] as u32) << 16) | ver[1] as u32;
    let ls = ((ver[2] as u32) << 16) | ver[3] as u32;
    dw(&mut fixed, ms);
    dw(&mut fixed, ls);
    dw(&mut fixed, ms);
    dw(&mut fixed, ls);
    dw(&mut fixed, 0x3F);
    dw(&mut fixed, 0);
    dw(&mut fixed, 0x0004_0004);
    dw(&mut fixed, 1);
    dw(&mut fixed, 0);
    dw(&mut fixed, 0);
    dw(&mut fixed, 0);
    assert_eq!(fixed.len(), 52);

    let ver_text = env!("CARGO_PKG_VERSION");
    let strings = [
        version_string("CompanyName", "Samek86"),
        version_string("FileDescription", "Maguro"),
        version_string("ProductName", "Maguro"),
        version_string("InternalName", "maguro"),
        version_string("OriginalFilename", "maguro.exe"),
        version_string("FileVersion", ver_text),
        version_string("ProductVersion", ver_text),
        version_string("Comments", "Fast multi-pane file manager for Windows"),
        version_string("LegalCopyright", "Copyright (c) 2026 Samek86. Based on efww/grids, Copyright (c) 2026 efww. MIT License."),
    ];
    let table = version_node("040904B0", &[], 0, true, &strings);
    let string_file = version_node("StringFileInfo", &[], 0, true, &[table]);
    let translation = {
        let mut v = Vec::with_capacity(4);
        v.extend(0x0409u16.to_le_bytes());
        v.extend(0x04B0u16.to_le_bytes());
        version_node("Translation", &v, 4, false, &[])
    };
    let var_file = version_node("VarFileInfo", &[], 0, false, &[translation]);
    version_node("VS_VERSION_INFO", &fixed, 52, false, &[string_file, var_file])
}

fn emit_git_hash() {
    // Short hash for About/title on debug and portable builds (see live_about).
    // Missing git is fine (empty env); CI checkouts without .git stay version-only.
    println!("cargo:rerun-if-env-changed=MAGURO_GIT_HASH");
    if let Ok(out) = std::process::Command::new("git")
        .args(["rev-parse", "--git-path", "HEAD"])
        .output()
    {
        if out.status.success() {
            if let Ok(p) = String::from_utf8(out.stdout) {
                let p = p.trim();
                if !p.is_empty() {
                    println!("cargo:rerun-if-changed={p}");
                }
            }
        }
    }
    let hash = std::env::var("MAGURO_GIT_HASH").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
        std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_default()
    });
    println!("cargo:rustc-env=MAGURO_GIT_HASH={hash}");
}
