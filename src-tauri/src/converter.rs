use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{command, AppHandle, Emitter};
use walkdir::WalkDir;

pub struct MobiCache(pub Mutex<HashMap<PathBuf, Vec<Vec<u8>>>>);
pub struct ConvertCancel(pub AtomicBool);

fn device_profile(device: &str) -> (u32, u32, &'static str) {
    match device {
        "kobo-clara-hd" => (1072, 1448, "kobo_clara_hd"),
        "kindle-paperwhite" => (1072, 1448, "kindle_pw"),
        "kindle-oasis" => (1264, 1680, "kindle_oasis"),
        "optimize" => (2048, 2048, "optimized"),
        "pdf-optimize" => (1500, 1500, "pdf_optimized"),
        _ => (600, 800, "kindle4"),
    }
}

fn is_cbz_output(device: &str) -> bool {
    device.starts_with("kobo") || device == "optimize"
}

/// Natural (human) order comparison: numeric runs compared by value, not
/// lexicographically, so `2.jpg` sorts before `10.jpg`. Text runs compared
/// case-insensitively. Keeps page order correct for non-zero-padded archives.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let mut na = String::new();
                    while let Some(c) = ai.peek().copied() {
                        if !c.is_ascii_digit() { break; }
                        na.push(c);
                        ai.next();
                    }
                    let mut nb = String::new();
                    while let Some(c) = bi.peek().copied() {
                        if !c.is_ascii_digit() { break; }
                        nb.push(c);
                        bi.next();
                    }
                    let ta = na.trim_start_matches('0');
                    let tb = nb.trim_start_matches('0');
                    let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                    if ord != Ordering::Equal { return ord; }
                    // Equal value: fewer leading zeros first, for stable order.
                    let ord = na.len().cmp(&nb.len());
                    if ord != Ordering::Equal { return ord; }
                } else {
                    let ord = ca
                        .to_ascii_lowercase()
                        .cmp(&cb.to_ascii_lowercase())
                        .then_with(|| ca.cmp(&cb));
                    if ord != Ordering::Equal { return ord; }
                    ai.next();
                    bi.next();
                }
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConvertOptions {
    pub input_path: String,
    pub output_dir: String,
    pub quality: u8,
    pub contrast: bool,
    pub no_split: bool,
    pub device: String,
    #[serde(default)]
    pub skip_existing: bool,
    #[serde(default)]
    pub preserve_color: bool,
    #[serde(default)]
    pub min_resolution: u32,
    #[serde(default)]
    pub max_image_dim: u32,
}

#[derive(Debug, Serialize, Clone)]
pub struct ConvertProgress {
    pub current: usize,
    pub total: usize,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ConvertResult {
    pub output_path: String,
    pub output_size: String,
    pub input_size: String,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub title: String,
    pub elapsed: String,
    pub skipped: bool,
    pub skip_reason: String,
}

#[derive(Debug, Serialize)]
pub struct MobiInfo {
    pub page_count: usize,
    pub file_size: String,
    pub title: String,
    pub author: String,
}

#[derive(Debug, Serialize)]
pub struct MobiPage {
    pub image: String,
    pub page: usize,
    pub page_count: usize,
}

#[command]
pub async fn check_is_dir(path: String) -> bool {
    PathBuf::from(&path).is_dir()
}

#[command]
pub async fn list_comics(dir: String) -> Result<Vec<String>, String> {
    let dir = PathBuf::from(&dir);
    if !dir.is_dir() {
        return Err("Not a directory".to_string());
    }
    let exts = ["cbr", "cbz", "rar", "zip", "pdf"];
    let mut comics: Vec<String> = WalkDir::new(&dir)
        .into_iter()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let ext = path.extension()?.to_str()?.to_lowercase();
            if exts.contains(&ext.as_str()) {
                Some(path.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect();
    comics.sort_by(|a, b| natural_cmp(a, b));
    Ok(comics)
}

fn extract_mobi_metadata(data: &[u8]) -> (String, String) {
    let pdb_title = std::str::from_utf8(&data[..32.min(data.len())])
        .unwrap_or("")
        .trim_end_matches('\0')
        .to_string();

    if data.len() < 132 {
        return (pdb_title, String::new());
    }

    let record0_offset = if data.len() >= 82 {
        u32::from_be_bytes([data[78], data[79], data[80], data[81]]) as usize
    } else {
        return (pdb_title, String::new());
    };

    if record0_offset + 132 > data.len() {
        return (pdb_title, String::new());
    }

    let rec = &data[record0_offset..];

    let full_title = if rec.len() >= 92 {
        let title_offset = u32::from_be_bytes([rec[84], rec[85], rec[86], rec[87]]) as usize;
        let title_len = u32::from_be_bytes([rec[88], rec[89], rec[90], rec[91]]) as usize;
        if title_len > 0 && title_offset.checked_add(title_len).map_or(false, |end| end <= rec.len()) {
            std::str::from_utf8(&rec[title_offset..title_offset + title_len])
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let mut author = String::new();
    if rec.len() > 20 {
        let header_len = u32::from_be_bytes([rec[20], rec[21], rec[22], rec[23]]) as usize;
        if let Some(exth_offset) = 16usize.checked_add(header_len) {
        if exth_offset + 12 <= rec.len() && &rec[exth_offset..exth_offset + 4] == b"EXTH" {
            let num_items = u32::from_be_bytes([
                rec[exth_offset + 8], rec[exth_offset + 9],
                rec[exth_offset + 10], rec[exth_offset + 11],
            ]) as usize;
            let mut pos = exth_offset + 12;
            for _ in 0..num_items.min(1000) {
                if pos + 8 > rec.len() { break; }
                let rec_type = u32::from_be_bytes([rec[pos], rec[pos+1], rec[pos+2], rec[pos+3]]);
                let rec_len = u32::from_be_bytes([rec[pos+4], rec[pos+5], rec[pos+6], rec[pos+7]]) as usize;
                if rec_len < 8 || pos + rec_len > rec.len() { break; }
                if rec_type == 100 {
                    author = std::str::from_utf8(&rec[pos+8..pos+rec_len])
                        .unwrap_or("")
                        .to_string();
                }
                pos += rec_len;
            }
        }
        }
    }

    let title = if !full_title.is_empty() { full_title } else { pdb_title };
    (title, author)
}

fn extract_images_from_mobi(path: &Path) -> Result<Vec<Vec<u8>>, String> {
    let data = fs::read(path).map_err(|e| format!("Cannot read MOBI: {e}"))?;
    extract_images_from_mobi_data(&data)
}

fn extract_images_from_mobi_data(data: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    if data.len() < 78 {
        return Err("File too small to be MOBI".to_string());
    }

    let num_records = u16::from_be_bytes([data[76], data[77]]) as usize;
    let mut record_offsets: Vec<usize> = Vec::with_capacity(num_records);

    for i in 0..num_records {
        let info_offset = 78 + i * 8;
        if info_offset + 4 > data.len() {
            break;
        }
        let offset = u32::from_be_bytes([
            data[info_offset],
            data[info_offset + 1],
            data[info_offset + 2],
            data[info_offset + 3],
        ]) as usize;
        record_offsets.push(offset);
    }

    let mut images = Vec::new();
    for (i, &offset) in record_offsets.iter().enumerate() {
        let end = if i + 1 < record_offsets.len() {
            record_offsets[i + 1]
        } else {
            data.len()
        };

        if offset >= data.len() || end > data.len() || offset >= end {
            continue;
        }

        let record = &data[offset..end];

        let is_image = (record.len() > 3 && record[0] == 0xFF && record[1] == 0xD8 && record[2] == 0xFF)
            || (record.len() > 4 && record[0..4] == [0x89, 0x50, 0x4E, 0x47])
            || (record.len() >= 6 && (&record[0..6] == b"GIF87a" || &record[0..6] == b"GIF89a"))
            || (record.len() > 2 && record[0] == 0x42 && record[1] == 0x4D);
        if is_image {
            images.push(record.to_vec());
        }
    }

    Ok(images)
}

fn extract_images_from_cbz(path: &Path) -> Result<Vec<Vec<u8>>, String> {
    let data = fs::read(path).map_err(|e| format!("Cannot read CBZ: {e}"))?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&data))
        .map_err(|e| format!("Cannot open CBZ: {e}"))?;

    let mut names: Vec<String> = (0..archive.len())
        .filter_map(|i| {
            let file = archive.by_index(i).ok()?;
            let name = file.name().to_string();
            let lower = name.to_lowercase();
            if lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png") || lower.ends_with(".webp") || lower.ends_with(".bmp") || lower.ends_with(".gif") || lower.ends_with(".tif") || lower.ends_with(".tiff") {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    names.sort_by(|a, b| natural_cmp(a, b));

    let mut images = Vec::new();
    for name in &names {
        if let Ok(mut file) = archive.by_name(name) {
            let mut buf = Vec::new();
            if file.read_to_end(&mut buf).is_ok() && !buf.is_empty() {
                images.push(buf);
            }
        }
    }
    Ok(images)
}

fn get_or_extract(cache: &MobiCache, path: &Path, preloaded: Option<&[u8]>) -> Result<Vec<Vec<u8>>, String> {
    {
        let map = cache.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(imgs) = map.get(path) {
            return Ok(imgs.clone());
        }
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let images = if ext == "cbz" || ext == "zip" {
        extract_images_from_cbz(path)?
    } else if let Some(data) = preloaded {
        extract_images_from_mobi_data(data)?
    } else {
        extract_images_from_mobi(path)?
    };
    let mut map = cache.0.lock().unwrap_or_else(|p| p.into_inner());
    if map.len() >= 3 {
        if let Some(oldest) = map.keys().next().cloned() {
            map.remove(&oldest);
        }
    }
    map.insert(path.to_path_buf(), images.clone());
    Ok(images)
}

#[command]
pub async fn get_mobi_info(
    path: String,
    cache: tauri::State<'_, MobiCache>,
) -> Result<MobiInfo, String> {
    let path = PathBuf::from(&path);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    if ext == "cbz" || ext == "zip" {
        let meta = fs::metadata(&path).map_err(|e| format!("Cannot read: {e}"))?;
        let title = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let images = get_or_extract(&cache, &path, None)?;
        Ok(MobiInfo {
            page_count: images.len(),
            file_size: format_size(meta.len() as usize),
            title,
            author: String::new(),
        })
    } else {
        let data = fs::read(&path).map_err(|e| format!("Cannot read MOBI: {e}"))?;
        let (title, author) = extract_mobi_metadata(&data);
        let file_size = format_size(data.len());
        let images = get_or_extract(&cache, &path, Some(&data))?;
        Ok(MobiInfo {
            page_count: images.len(),
            file_size,
            title,
            author,
        })
    }
}

#[command]
pub async fn get_mobi_page(
    path: String,
    page: usize,
    cache: tauri::State<'_, MobiCache>,
) -> Result<MobiPage, String> {
    let path = PathBuf::from(&path);
    let images = get_or_extract(&cache, &path, None)?;

    if images.is_empty() {
        return Err("No images found".to_string());
    }

    let idx = page.min(images.len() - 1);
    let mime = match image_ext_from_bytes(&images[idx]) {
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        _ => "image/jpeg",
    };
    let b64 = {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&images[idx]);
        format!("data:{mime};base64,{encoded}")
    };

    Ok(MobiPage {
        image: b64,
        page: idx,
        page_count: images.len(),
    })
}

fn detect_archive_type(path: &Path) -> &'static str {
    if let Ok(mut f) = fs::File::open(path) {
        let mut buf = [0u8; 8];
        let n = f.read(&mut buf).unwrap_or(0);
        if n >= 4 && buf[0] == 0x50 && buf[1] == 0x4B {
            return "zip";
        }
        if n >= 7 && &buf[0..7] == b"Rar!\x1a\x07\x00" {
            return "rar";
        }
        if n >= 8 && &buf[0..8] == b"Rar!\x1a\x07\x01\x00" {
            return "rar";
        }
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "cbz" | "zip" => "zip",
        "cbr" | "rar" => "rar",
        _ => "unknown",
    }
}

fn is_zip_out_of_order(names: &[String]) -> bool {
    let mut sorted = names.to_vec();
    sorted.sort_by(|a, b| natural_cmp(a, b));
    names != sorted
}

fn extract_archive_to_dir(input_path: &Path, dest_dir: &Path) -> Result<(usize, bool), String> {
    use std::io::Read;

    let archive_type = detect_archive_type(input_path);

    if archive_type == "zip" {
        let data = fs::read(input_path).map_err(|e| format!("Cannot read: {e}"))?;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&data))
            .map_err(|e| format!("Cannot open CBZ: {e}"))?;

        let mut names: Vec<String> = (0..archive.len())
            .filter_map(|i| {
                let file = archive.by_index(i).ok()?;
                let name = file.name().to_string();
                let lower = name.to_lowercase();
                if lower.ends_with(".jpg")
                    || lower.ends_with(".jpeg")
                    || lower.ends_with(".png")
                    || lower.ends_with(".webp")
                    || lower.ends_with(".bmp")
                    || lower.ends_with(".gif")
                    || lower.ends_with(".tif")
                    || lower.ends_with(".tiff")
                {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        let reordered = is_zip_out_of_order(&names);
        names.sort_by(|a, b| natural_cmp(a, b));

        let mut count = 0;
        for name in &names {
            if let Ok(mut file) = archive.by_name(name) {
                let mut buf = Vec::new();
                if file.read_to_end(&mut buf).is_ok() && !buf.is_empty() {
                    let ext = image_ext_from_bytes(&buf);
                    let out_name = format!("{:04}.{ext}", count);
                    if fs::write(dest_dir.join(&out_name), &buf).is_ok() {
                        count += 1;
                    }
                }
            }
        }
        Ok((count, reordered))
    } else if archive_type == "rar" {
        #[cfg(windows)]
        hide_console_window();

        let unrar = find_unrar().ok_or_else(|| {
            "unrar not found — install it with: brew install unrar (macOS) or apt install unrar (Linux)".to_string()
        })?;

        let status = std::process::Command::new(&unrar)
            .args(["e", "-o+", "-inul", "--"])
            .arg(input_path)
            .arg(dest_dir)
            .status()
            .map_err(|e| format!("Failed to run unrar: {e}"))?;

        if !status.success() {
            let retry = std::process::Command::new(&unrar)
                .args(["e", "-o+", "--"])
                .arg(input_path)
                .arg(dest_dir)
                .output()
                .map_err(|e| format!("Failed to run unrar: {e}"))?;
            if !retry.status.success() {
                let stderr = String::from_utf8_lossy(&retry.stderr);
                return Err(format!("unrar failed: {}", stderr.trim()));
            }
        }

        let mut images: Vec<PathBuf> = fs::read_dir(dest_dir)
            .map_err(|e| format!("Cannot read temp dir: {e}"))?
            .filter_map(|e| {
                let path = e.ok()?.path();
                let ext = path.extension()?.to_str()?.to_lowercase();
                if ["jpg", "jpeg", "png", "webp", "bmp", "gif", "tif", "tiff"].contains(&ext.as_str()) {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();

        // Sort by archive listing order (unrar lb preserves source order)
        if let Ok(list_output) = std::process::Command::new(&unrar)
            .args(["lb", "--"])
            .arg(input_path)
            .output()
        {
            if list_output.status.success() {
                let archive_order: HashMap<String, usize> = String::from_utf8_lossy(&list_output.stdout)
                    .lines()
                    .filter_map(|line| {
                        let lower = line.to_lowercase();
                        if lower.ends_with(".jpg") || lower.ends_with(".jpeg")
                            || lower.ends_with(".png") || lower.ends_with(".webp")
                            || lower.ends_with(".bmp") || lower.ends_with(".gif")
                            || lower.ends_with(".tif") || lower.ends_with(".tiff")
                        {
                            Path::new(line).file_name()
                                .map(|n| n.to_string_lossy().to_string())
                        } else {
                            None
                        }
                    })
                    .enumerate()
                    .map(|(i, name)| (name, i))
                    .collect();
                images.sort_by_key(|p| {
                    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                    *archive_order.get(&name).unwrap_or(&usize::MAX)
                });
            }
        }

        let count = images.len();
        // Two-phase rename to avoid collisions when source names overlap with target {:04} names
        for (i, img_path) in images.iter().enumerate() {
            let ext = img_path.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
            let tmp_name = format!("__tmp_{:04}.{ext}", i);
            let _ = fs::rename(img_path, dest_dir.join(&tmp_name));
        }
        for i in 0..count {
            let pattern = format!("__tmp_{:04}.", i);
            if let Ok(entries) = fs::read_dir(dest_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(&pattern) {
                        let ext = entry.path().extension().and_then(|e| e.to_str()).unwrap_or("jpg").to_string();
                        let _ = fs::rename(entry.path(), dest_dir.join(format!("{:04}.{ext}", i)));
                        break;
                    }
                }
            }
        }
        Ok((count, false))
    } else {
        Err(format!("Unsupported archive format: {archive_type}"))
    }
}

fn find_unrar() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in ["UnRAR.exe", "unrar.exe"] {
                let p = dir.join(name);
                if p.exists() {
                    return Some(p);
                }
                let p = dir.join("resources").join(name);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }

    let candidates = if cfg!(windows) {
        vec![
            r"C:\Program Files\WinRAR\UnRAR.exe".to_string(),
            r"C:\Program Files (x86)\WinRAR\UnRAR.exe".to_string(),
            "unrar.exe".to_string(),
        ]
    } else {
        vec![
            "/usr/local/bin/unrar".to_string(),
            "/opt/homebrew/bin/unrar".to_string(),
            "/usr/bin/unrar".to_string(),
            "unrar".to_string(),
        ]
    };
    for c in candidates {
        let p = PathBuf::from(&c);
        if p.is_absolute() && p.exists() {
            return Some(p);
        }
        if std::process::Command::new(&c)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
        {
            return Some(PathBuf::from(c));
        }
    }
    None
}

/// Run a child process to completion while honoring the cancel flag: polls
/// for exit and kills the child if a cancel is requested mid-render. Returns
/// whether the process exited successfully.
fn run_with_cancel(
    mut cmd: std::process::Command,
    cancel: &AtomicBool,
    tool: &str,
) -> Result<bool, String> {
    let mut child = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to run {tool}: {e}"))?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Cancelled".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return Err(format!("{tool} failed: {e}")),
        }
    }
}

fn render_pdf_to_dir(input_path: &Path, dest_dir: &Path, cancel: &AtomicBool) -> Result<usize, String> {
    #[cfg(windows)]
    hide_console_window();

    if let Some(mutool) = find_tool(&[
        "/opt/homebrew/bin/mutool",
        "/usr/local/bin/mutool",
        "mutool",
    ]) {
        let pattern = dest_dir.join("page_%04d.png");
        let mut cmd = std::process::Command::new(&mutool);
        cmd.args(["draw", "-o"])
            .arg(&pattern)
            .args(["-r", "150"])
            .arg(input_path);
        if run_with_cancel(cmd, cancel, "mutool")? {
            return collect_rendered_images(dest_dir);
        }
    }

    if let Some(pdftoppm) = find_tool(&[
        "/opt/homebrew/bin/pdftoppm",
        "/usr/local/bin/pdftoppm",
        "pdftoppm",
    ]) {
        let prefix = dest_dir.join("page");
        let mut cmd = std::process::Command::new(&pdftoppm);
        cmd.args(["-png", "-r", "150"])
            .arg(input_path)
            .arg(&prefix);
        if run_with_cancel(cmd, cancel, "pdftoppm")? {
            return collect_rendered_images(dest_dir);
        }
    }

    Err("No PDF renderer found — install mupdf-tools or poppler".to_string())
}

fn find_tool(candidates: &[&str]) -> Option<PathBuf> {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for &name in candidates {
                if let Some(basename) = Path::new(name).file_name() {
                    let base_str = basename.to_string_lossy();
                    let names: Vec<std::ffi::OsString> = if !base_str.contains('.') {
                        vec![basename.to_os_string(), std::ffi::OsString::from(format!("{base_str}.exe"))]
                    } else {
                        vec![basename.to_os_string()]
                    };
                    for n in &names {
                        let p = dir.join(n);
                        if p.exists() {
                            return Some(p);
                        }
                        let p = dir.join("resources").join(n);
                        if p.exists() {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }

    for &name in candidates {
        let p = PathBuf::from(name);
        if p.is_absolute() && p.exists() {
            return Some(p);
        }
        if std::process::Command::new(name)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
        {
            return Some(p);
        }
    }
    None
}

fn collect_rendered_images(dir: &Path) -> Result<usize, String> {
    let mut images: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("Cannot read dir: {e}"))?
        .filter_map(|e| {
            let path = e.ok()?.path();
            let ext = path.extension()?.to_str()?.to_lowercase();
            if ["png", "jpg", "jpeg", "ppm"].contains(&ext.as_str()) {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    if images.is_empty() {
        return Err("PDF rendering produced no images".to_string());
    }

    images.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    let count = images.len();
    for (i, img_path) in images.iter().enumerate() {
        let ext = img_path.extension().and_then(|e| e.to_str()).unwrap_or("png");
        let tmp_name = format!("__tmp_{:04}.{ext}", i);
        let _ = fs::rename(img_path, dir.join(&tmp_name));
    }
    for i in 0..count {
        let pattern = format!("__tmp_{:04}.", i);
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&pattern) {
                    let ext = entry.path().extension().and_then(|e| e.to_str()).unwrap_or("png").to_string();
                    let _ = fs::rename(entry.path(), dir.join(format!("{:04}.{ext}", i)));
                    break;
                }
            }
        }
    }
    Ok(count)
}

fn peek_image_dimensions(raw: &[u8]) -> Option<(u32, u32)> {
    use image::ImageReader;
    use std::io::Cursor;
    ImageReader::new(Cursor::new(raw))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

fn check_source_quality(input_path: &Path, target: u32, split: bool) -> Option<(u32, u32, bool)> {
    use std::io::Read;

    let peek_mid = |buf: &[u8]| -> Option<(u32, u32, bool)> {
        let (w, h) = peek_image_dimensions(buf)?;
        // Match process_image_split: a page is only halved when it is wide
        // enough (w > h * 5/4), not merely landscape (w > h).
        let will_split = split && w > h * 5 / 4;
        let effective_w = if will_split { w / 2 } else { w };
        let too_small = target > 0 && (effective_w < target || h < target);
        if too_small {
            Some((w, h, will_split))
        } else {
            None
        }
    };

    let archive_type = detect_archive_type(input_path);
    if archive_type == "zip" {
        let data = fs::read(input_path).ok()?;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&data)).ok()?;
        let mut names: Vec<String> = (0..archive.len())
            .filter_map(|i| {
                let file = archive.by_index(i).ok()?;
                let name = file.name().to_string();
                let lower = name.to_lowercase();
                if lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png") || lower.ends_with(".webp") || lower.ends_with(".bmp") || lower.ends_with(".gif") || lower.ends_with(".tif") || lower.ends_with(".tiff") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        names.sort_by(|a, b| natural_cmp(a, b));
        if names.is_empty() { return None; }
        let mid = names.len() / 2;
        let mut file = archive.by_name(&names[mid]).ok()?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).ok()?;
        peek_mid(&buf)
    } else if archive_type == "rar" {
        let tmp_dir = tempfile::TempDir::new().ok()?;
        let unrar = find_unrar()?;
        let status = std::process::Command::new(&unrar)
            .args(["e", "-o+", "-inul", "--"])
            .arg(input_path)
            .arg(tmp_dir.path())
            .status()
            .ok()?;
        if !status.success() { return None; }
        let mut imgs: Vec<PathBuf> = fs::read_dir(tmp_dir.path())
            .ok()?
            .filter_map(|e| {
                let path = e.ok()?.path();
                let ext = path.extension()?.to_str()?.to_lowercase();
                if ["jpg", "jpeg", "png", "webp", "bmp", "gif", "tif", "tiff"].contains(&ext.as_str()) {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();
        imgs.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
        if imgs.is_empty() { return None; }
        let mid = imgs.len() / 2;
        let data = fs::read(&imgs[mid]).ok()?;
        peek_mid(&data)
    } else {
        None
    }
}

fn check_optimize_savings(
    input_path: &Path,
    width: u32,
    height: u32,
    quality: u8,
    contrast: bool,
    grayscale: bool,
) -> Option<String> {
    use std::io::Read;

    let archive_type = detect_archive_type(input_path);
    let sample_count = 5usize;

    let samples: Vec<(usize, Vec<u8>)> = if archive_type == "zip" {
        let data = fs::read(input_path).ok()?;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&data)).ok()?;
        let mut names: Vec<String> = (0..archive.len())
            .filter_map(|i| {
                let file = archive.by_index(i).ok()?;
                let name = file.name().to_string();
                let lower = name.to_lowercase();
                if is_image_ext(&lower) { Some(name) } else { None }
            })
            .collect();
        names.sort_by(|a, b| natural_cmp(a, b));
        if names.is_empty() { return None; }
        let indices = pick_sample_indices(names.len(), sample_count);
        indices.iter().filter_map(|&i| {
            let mut file = archive.by_name(&names[i]).ok()?;
            let compressed = file.compressed_size() as usize;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).ok()?;
            Some((compressed, buf))
        }).collect()
    } else if archive_type == "rar" {
        let tmp_dir = tempfile::TempDir::new().ok()?;
        let unrar = find_unrar()?;
        let status = std::process::Command::new(&unrar)
            .args(["e", "-o+", "-inul", "--"])
            .arg(input_path)
            .arg(tmp_dir.path())
            .status()
            .ok()?;
        if !status.success() { return None; }
        let mut imgs: Vec<PathBuf> = fs::read_dir(tmp_dir.path())
            .ok()?
            .filter_map(|e| {
                let path = e.ok()?.path();
                let ext = path.extension()?.to_str()?.to_lowercase();
                if is_image_ext(&ext) { Some(path) } else { None }
            })
            .collect();
        imgs.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
        if imgs.is_empty() { return None; }
        let indices = pick_sample_indices(imgs.len(), sample_count);
        indices.iter().filter_map(|&i| {
            let size = fs::metadata(&imgs[i]).ok()?.len() as usize;
            let data = fs::read(&imgs[i]).ok()?;
            Some((size, data))
        }).collect()
    } else {
        return None;
    };

    if samples.is_empty() { return None; }

    let mut total_source: usize = 0;
    let mut total_encoded: usize = 0;
    let mut fmt_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (source_size, buf) in &samples {
        let fmt = image_ext_from_bytes(buf);
        *fmt_counts.entry(fmt.to_string()).or_insert(0) += 1;
        let img = match decode_image(buf) {
            Ok(img) => img,
            Err(_) => continue,
        };
        let resize = img.width() > width || img.height() > height;
        let encoded = match encode_image(img, width, height, quality, contrast, grayscale, resize) {
            Ok(e) => e,
            Err(_) => continue,
        };
        total_source += source_size;
        total_encoded += encoded.len();
    }

    if total_source == 0 { return None; }

    let ratio_pct = total_encoded * 100 / total_source;
    let savings_pct = if ratio_pct >= 100 { 0usize } else { 100 - ratio_pct };
    if savings_pct < 15 {
        let dominant_fmt = fmt_counts.iter().max_by_key(|(_, c)| *c).map(|(f, _)| f.as_str()).unwrap_or("?");
        Some(format!("already compact ({dominant_fmt}, ~{savings_pct}% savings across {}/{} samples)", samples.len(), sample_count))
    } else {
        None
    }
}

fn is_image_ext(s: &str) -> bool {
    matches!(s.rsplit('.').next().unwrap_or(""),
        "jpg" | "jpeg" | "png" | "webp" | "bmp" | "gif" | "tif" | "tiff")
}

fn pick_sample_indices(total: usize, count: usize) -> Vec<usize> {
    let n = count.min(total);
    (0..n).map(|i| i * total / n).collect()
}

fn decode_image(raw: &[u8]) -> Result<image::DynamicImage, String> {
    use image::ImageReader;
    use std::io::Cursor;
    ImageReader::new(Cursor::new(raw))
        .with_guessed_format()
        .map_err(|e| format!("Image format error: {e}"))?
        .decode()
        .map_err(|e| format!("Decode error: {e}"))
}

fn encode_image(
    img: image::DynamicImage,
    width: u32,
    height: u32,
    quality: u8,
    contrast: bool,
    grayscale: bool,
    resize: bool,
) -> Result<Vec<u8>, String> {
    use image::imageops::FilterType;
    use std::io::Cursor;

    let filter = if width >= 2048 { FilterType::Triangle } else { FilterType::CatmullRom };
    let img = if resize && (img.width() > width || img.height() > height) {
        img.resize(width, height, filter)
    } else {
        img
    };
    let img = if grayscale {
        image::DynamicImage::ImageLuma8(img.to_luma8())
    } else {
        img
    };
    let img = if contrast {
        if grayscale {
            image::DynamicImage::ImageLuma8(image::imageops::contrast(&img.to_luma8(), 20.0))
        } else {
            image::DynamicImage::ImageRgba8(image::imageops::contrast(&img.to_rgba8(), 20.0))
        }
    } else {
        img
    };

    let mut jpeg_buf = Cursor::new(Vec::new());
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, quality);
    img.write_with_encoder(encoder)
        .map_err(|e| format!("JPEG encode error: {e}"))?;
    Ok(jpeg_buf.into_inner())
}

fn process_image_split(
    raw: &[u8],
    width: u32,
    height: u32,
    quality: u8,
    contrast: bool,
    grayscale: bool,
    resize: bool,
    split: bool,
) -> Result<Vec<Vec<u8>>, String> {
    let img = decode_image(raw)?;
    let (w, h) = (img.width(), img.height());
    let pages = if split && w > h * 5 / 4 {
        let mid = w / 2;
        vec![img.crop_imm(0, 0, mid, h), img.crop_imm(mid, 0, w - mid, h)]
    } else {
        vec![img]
    };
    pages.into_iter()
        .map(|page| encode_image(page, width, height, quality, contrast, grayscale, resize))
        .collect()
}

fn optimize_dir_to_cbz(
    dir: &Path,
    output_path: &Path,
    width: u32,
    height: u32,
    quality: u8,
    contrast: bool,
    grayscale: bool,
    resize: bool,
    split: bool,
    cancel: &AtomicBool,
    app: &AppHandle,
) -> Result<(), String> {
    use rayon::prelude::*;
    use std::io::Write;

    let mut images: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("Cannot read directory: {e}"))?
        .filter_map(|e| {
            let path = e.ok()?.path();
            let ext = path.extension()?.to_str()?.to_lowercase();
            if ["jpg", "jpeg", "png", "webp", "ppm", "bmp", "gif", "tif", "tiff"].contains(&ext.as_str()) {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    images.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));

    if images.is_empty() {
        return Err("No images found".to_string());
    }

    let total = images.len();
    emit_progress(app, 0, total, &format!("Processing 0/{total}"));

    let raw_images: Vec<Vec<u8>> = images.iter()
        .map(|p| {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            fs::read(p).map_err(|e| format!("Cannot read image: {e}"))
        })
        .collect::<Result<_, _>>()?;

    let done = std::sync::atomic::AtomicUsize::new(0);
    let processed: Vec<Result<Vec<Vec<u8>>, String>> = raw_images.par_iter()
        .map(|raw| {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            let result = process_image_split(raw, width, height, quality, contrast, grayscale, resize, split);
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            emit_progress(app, n, total, &format!("Processing {n}/{total}"));
            result
        })
        .collect();

    let out_file = fs::File::create(output_path).map_err(|e| format!("Cannot create output: {e}"))?;
    let mut zip_writer = zip::ZipWriter::new(out_file);
    let zip_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    let mut page_idx = 0;
    for (i, result) in processed.into_iter().enumerate() {
        let pages = result?;
        emit_progress(app, i + 1, total, &format!("Writing {}/{total}", i + 1));
        for jpeg_data in pages {
            let out_name = format!("{:04}.jpg", page_idx);
            zip_writer.start_file(&out_name, zip_options)
                .map_err(|e| format!("ZIP write error: {e}"))?;
            zip_writer.write_all(&jpeg_data)
                .map_err(|e| format!("ZIP write error: {e}"))?;
            page_idx += 1;
        }
    }

    zip_writer.finish().map_err(|e| format!("ZIP finalize error: {e}"))?;
    Ok(())
}

#[command]
pub async fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[command]
pub async fn cancel_convert(cancel: tauri::State<'_, ConvertCancel>) -> Result<(), String> {
    cancel.0.store(true, Ordering::Relaxed);
    Ok(())
}

#[command]
pub async fn reset_cancel(cancel: tauri::State<'_, ConvertCancel>) -> Result<(), String> {
    cancel.0.store(false, Ordering::Relaxed);
    Ok(())
}

fn optimize_cbz(
    input_path: &Path,
    output_path: &Path,
    width: u32,
    height: u32,
    quality: u8,
    contrast: bool,
    grayscale: bool,
    resize: bool,
    split: bool,
    cancel: &AtomicBool,
    app: &AppHandle,
) -> Result<(), String> {
    use rayon::prelude::*;
    use std::io::{Cursor, Read, Write};

    let archive_type = detect_archive_type(input_path);

    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();

    if archive_type == "zip" {
        let data = fs::read(input_path).map_err(|e| format!("Cannot read input: {e}"))?;
        let reader = zip::ZipArchive::new(Cursor::new(&data))
            .map_err(|e| format!("Cannot open archive: {e}"))?;
        let mut archive = reader;
        let mut names: Vec<String> = (0..archive.len())
            .filter_map(|i| {
                let file = archive.by_index(i).ok()?;
                let name = file.name().to_string();
                let lower = name.to_lowercase();
                if lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png") || lower.ends_with(".webp") || lower.ends_with(".bmp") || lower.ends_with(".gif") || lower.ends_with(".tif") || lower.ends_with(".tiff") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        if is_zip_out_of_order(&names) {
            emit_progress(app, 0, 1, "Source archive had out-of-order pages, reordering...");
        }
        names.sort_by(|a, b| natural_cmp(a, b));

        for name in &names {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            let mut file = archive.by_name(name).map_err(|e| format!("Cannot read {name}: {e}"))?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).map_err(|e| format!("Read error: {e}"))?;
            entries.push((name.clone(), buf));
        }
    } else if archive_type == "rar" {
        let tmp_dir = tempfile::TempDir::new()
            .map_err(|e| format!("Cannot create temp dir: {e}"))?;
        let (count, _) = extract_archive_to_dir(input_path, tmp_dir.path())?;
        if count == 0 {
            return Err("No images found in archive".to_string());
        }
        let mut imgs: Vec<PathBuf> = fs::read_dir(tmp_dir.path())
            .map_err(|e| format!("Cannot read temp dir: {e}"))?
            .filter_map(|e| {
                let path = e.ok()?.path();
                let ext = path.extension()?.to_str()?.to_lowercase();
                if ["jpg", "jpeg", "png", "webp", "bmp", "gif", "tif", "tiff"].contains(&ext.as_str()) {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();
        imgs.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
        for img_path in &imgs {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            if let Ok(data) = fs::read(img_path) {
                let name = img_path.file_name().unwrap_or_default().to_string_lossy().to_string();
                entries.push((name, data));
            }
        }
    } else {
        return Err(format!("Unsupported archive format: {archive_type}"));
    }

    if entries.is_empty() {
        return Err("No images found in archive".to_string());
    }

    let total = entries.len();
    emit_progress(app, 0, total, &format!("Processing 0/{total}"));

    let done = std::sync::atomic::AtomicUsize::new(0);
    let processed: Vec<Result<Vec<Vec<u8>>, String>> = entries.par_iter()
        .map(|(_name, raw)| {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            let result = process_image_split(raw, width, height, quality, contrast, grayscale, resize, split);
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            emit_progress(app, n, total, &format!("Processing {n}/{total}"));
            result
        })
        .collect();

    let out_file = fs::File::create(output_path).map_err(|e| format!("Cannot create output: {e}"))?;
    let mut zip_writer = zip::ZipWriter::new(out_file);
    let zip_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    let mut page_idx = 0;
    for (i, result) in processed.into_iter().enumerate() {
        let pages = result?;
        emit_progress(app, i + 1, total, &format!("Writing {}/{total}", i + 1));
        for jpeg_data in pages {
            let out_name = format!("{:04}.jpg", page_idx);
            zip_writer.start_file(&out_name, zip_options)
                .map_err(|e| format!("ZIP write error: {e}"))?;
            zip_writer.write_all(&jpeg_data)
                .map_err(|e| format!("ZIP write error: {e}"))?;
            page_idx += 1;
        }
    }

    zip_writer.finish().map_err(|e| format!("ZIP finalize error: {e}"))?;
    Ok(())
}

fn optimize_pdf_file(
    input_path: &Path,
    output_path: &Path,
    quality: u8,
    grayscale: bool,
    max_dim: u32,
    cancel: &AtomicBool,
    app: &AppHandle,
) -> Result<(), String> {
    // Cheap metadata read (~50ms even on huge PDFs) so the slow full load that
    // follows shows the page count instead of looking hung. lopdf's full load
    // is single-threaded and uncancellable; a large book can take many seconds.
    let page_hint = lopdf::Document::load_metadata(input_path)
        .map(|m| m.page_count)
        .unwrap_or(0);
    if page_hint >= 500 {
        emit_progress(app, 0, 1, &format!("Loading PDF ({page_hint} pages, this can take a while)..."));
    } else {
        emit_progress(app, 0, 1, "Loading PDF...");
    }

    let mut doc = lopdf::Document::load(input_path)
        .map_err(|e| format!("Cannot load PDF: {e}"))?;

    // lopdf only decrypts RC4 revisions 2-3, not AES-256 (rev 5/6). Encrypted
    // image streams would decode as garbage and silently optimize nothing, so
    // decrypt via mutool into a temp PDF first, then optimize that. Keep the
    // temp dir alive (`_decrypt_tmp`) until the function returns.
    let _decrypt_tmp;
    if doc.trailer.get(b"Encrypt").is_ok() {
        emit_progress(app, 0, 1, "Decrypting PDF...");
        let mutool = find_tool(&[
            "/opt/homebrew/bin/mutool",
            "/usr/local/bin/mutool",
            "mutool",
        ])
        .ok_or("PDF is encrypted and mutool (mupdf-tools) is unavailable to decrypt it")?;

        let dir = tempfile::TempDir::new().map_err(|e| format!("Cannot create temp dir: {e}"))?;
        let decrypted = dir.path().join("decrypted.pdf");
        let mut cmd = std::process::Command::new(&mutool);
        cmd.arg("convert").arg("-o").arg(&decrypted).arg(input_path);
        if !run_with_cancel(cmd, cancel, "mutool")? {
            return Err("Failed to decrypt PDF (mutool convert failed)".to_string());
        }
        doc = lopdf::Document::load(&decrypted)
            .map_err(|e| format!("Cannot load decrypted PDF: {e}"))?;
        _decrypt_tmp = Some(dir);
    } else {
        _decrypt_tmp = None;
    }

    let image_ids: Vec<lopdf::ObjectId> = doc
        .objects
        .iter()
        .filter_map(|(id, obj)| {
            if let lopdf::Object::Stream(stream) = obj {
                let subtype = stream
                    .dict
                    .get(b"Subtype")
                    .ok()
                    .and_then(|o| o.as_name().ok())
                    .unwrap_or(b"");
                if subtype == b"Image" {
                    return Some(*id);
                }
            }
            None
        })
        .collect();

    let total = image_ids.len();
    if total == 0 {
        doc.save(output_path).map_err(|e| format!("Cannot save PDF: {e}"))?;
        return Ok(());
    }

    emit_progress(app, 0, total, &format!("Found {total} images in PDF"));

    let mut optimized = 0usize;
    let mut saved_bytes: i64 = 0;

    for (i, id) in image_ids.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".to_string());
        }

        match recompress_pdf_image(&mut doc, *id, quality, grayscale, max_dim) {
            Ok(diff) if diff > 0 => {
                optimized += 1;
                saved_bytes += diff;
            }
            _ => {}
        }

        emit_progress(app, i + 1, total, &format!("Optimizing {}/{total}", i + 1));
    }

    emit_progress(
        app,
        total,
        total,
        &format!(
            "Saving PDF ({optimized} images optimized, saved ~{})",
            format_size(saved_bytes.max(0) as usize)
        ),
    );

    doc.save(output_path).map_err(|e| format!("Cannot save PDF: {e}"))?;
    Ok(())
}

fn recompress_pdf_image(
    doc: &mut lopdf::Document,
    id: lopdf::ObjectId,
    quality: u8,
    grayscale: bool,
    max_dim: u32,
) -> Result<i64, String> {
    let (width, height, filter_bytes, bpc, cs_name, original_len) = {
        let stream = match doc.objects.get(&id) {
            Some(lopdf::Object::Stream(s)) => s,
            _ => return Err("Not a stream".to_string()),
        };

        let w = stream
            .dict
            .get(b"Width")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .filter(|&v| v > 0 && v <= 65535)
            .unwrap_or(0) as u32;
        let h = stream
            .dict
            .get(b"Height")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .filter(|&v| v > 0 && v <= 65535)
            .unwrap_or(0) as u32;
        let filter = stream
            .dict
            .get(b"Filter")
            .ok()
            .and_then(|o| o.as_name().ok())
            .map(|n| n.to_vec());
        let bpc = stream
            .dict
            .get(b"BitsPerComponent")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(8) as u32;
        let cs = stream
            .dict
            .get(b"ColorSpace")
            .ok()
            .and_then(|o| o.as_name().ok())
            .map(|n| n.to_vec());

        (w, h, filter, bpc, cs, stream.content.len())
    };

    if width < 64 || height < 64 {
        return Ok(0);
    }

    let filter_name = filter_bytes.as_deref();

    let img = match filter_name {
        Some(b"DCTDecode") => {
            let jpeg_data = match doc.objects.get(&id) {
                Some(lopdf::Object::Stream(s)) => s.content.clone(),
                _ => return Err("Not a stream".to_string()),
            };
            decode_image(&jpeg_data)?
        }
        Some(b"FlateDecode") => {
            if bpc != 8 {
                return Err(format!("Unsupported BPC: {bpc}"));
            }
            let channels: u32 = match cs_name.as_deref() {
                Some(b"DeviceRGB") => 3,
                Some(b"DeviceGray") => 1,
                _ => return Err("Unsupported color space".to_string()),
            };

            let raw = {
                let stream = match doc.objects.get(&id) {
                    Some(lopdf::Object::Stream(s)) => s,
                    _ => return Err("Not a stream".to_string()),
                };
                let mut cloned = stream.clone();
                let _ = cloned.decompress();
                cloned.content
            };

            let expected = (width as usize)
                .checked_mul(height as usize)
                .and_then(|x| x.checked_mul(channels as usize))
                .ok_or_else(|| "Image dimensions overflow".to_string())?;
            if raw.len() < expected {
                return Err("Data too short".to_string());
            }

            if channels == 3 {
                image::DynamicImage::ImageRgb8(
                    image::RgbImage::from_raw(width, height, raw[..expected].to_vec())
                        .ok_or("Cannot create RGB image")?,
                )
            } else {
                image::DynamicImage::ImageLuma8(
                    image::GrayImage::from_raw(width, height, raw[..expected].to_vec())
                        .ok_or("Cannot create gray image")?,
                )
            }
        }
        _ => return Err("Unsupported filter".to_string()),
    };

    let needs_resize = max_dim > 0 && (img.width() > max_dim || img.height() > max_dim);
    let (new_w, new_h) = if needs_resize {
        let ratio =
            (max_dim as f64 / img.width() as f64).min(max_dim as f64 / img.height() as f64);
        (
            (img.width() as f64 * ratio).round().max(1.0) as u32,
            (img.height() as f64 * ratio).round().max(1.0) as u32,
        )
    } else {
        (img.width(), img.height())
    };

    let encoded = encode_image(img, max_dim, max_dim, quality, false, grayscale, needs_resize)?;

    if encoded.len() >= original_len {
        return Ok(0);
    }

    let diff = original_len as i64 - encoded.len() as i64;

    if let Some(lopdf::Object::Stream(ref mut s)) = doc.objects.get_mut(&id) {
        let new_len = encoded.len();
        s.content = encoded;
        // Keep /Length in sync with the replaced content, else strict readers
        // (Preview, Adobe) flag "stream Length incorrect".
        s.dict.set("Length", lopdf::Object::Integer(new_len as i64));
        s.dict.set("Filter", lopdf::Object::Name(b"DCTDecode".to_vec()));
        s.dict.remove(b"DecodeParms");
        if needs_resize {
            s.dict
                .set("Width", lopdf::Object::Integer(new_w as i64));
            s.dict
                .set("Height", lopdf::Object::Integer(new_h as i64));
        }
        if grayscale {
            s.dict.set(
                "ColorSpace",
                lopdf::Object::Name(b"DeviceGray".to_vec()),
            );
        }
        s.allows_compression = false;
    }

    Ok(diff)
}

#[command]
pub async fn convert_comic(
    app: AppHandle,
    options: ConvertOptions,
    cache: tauri::State<'_, MobiCache>,
    cancel: tauri::State<'_, ConvertCancel>,
) -> Result<ConvertResult, String> {
    let input_path = PathBuf::from(&options.input_path);
    let output_dir = fs::canonicalize(&options.output_dir)
        .unwrap_or_else(|_| PathBuf::from(&options.output_dir));

    let title = input_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .trim_end_matches('.')
        .trim()
        .to_string();

    let (dev_w, dev_h, dev_name) = device_profile(&options.device);
    let is_optimize = options.device == "optimize" || options.device == "pdf-optimize";

    let output_ext = if options.device == "pdf-optimize" { "pdf" } else if is_cbz_output(&options.device) { "cbz" } else { "mobi" };
    let base_name = if is_optimize {
        format!("{title}_optimized")
    } else {
        title.clone()
    };
    let expected_output = output_dir.join(format!("{base_name}.{output_ext}"));

    if options.skip_existing && expected_output.exists() {
        let output_bytes = fs::metadata(&expected_output).map(|m| m.len() as usize).unwrap_or(0);
        let input_bytes = fs::metadata(&input_path).map(|m| m.len() as usize).unwrap_or(0);
        return Ok(ConvertResult {
            output_path: expected_output.to_string_lossy().to_string(),
            output_size: format_size(output_bytes),
            input_size: format_size(input_bytes),
            input_bytes,
            output_bytes,
            title,
            elapsed: "0.0s".to_string(),
            skipped: true,
            skip_reason: "exists".to_string(),
        });
    }

    let expected_output = if is_optimize && expected_output.exists() {
        let mut counter = 1;
        loop {
            let candidate = output_dir.join(format!("{base_name}_{counter}.{output_ext}"));
            if !candidate.exists() {
                break candidate;
            }
            counter += 1;
            if counter > 9999 {
                return Err("Too many files with the same name in output folder".to_string());
            }
        }
    } else {
        expected_output
    };

    cache.0.lock().unwrap_or_else(|p| p.into_inner()).remove(&input_path);

    let quality = options.quality.clamp(1, 100);
    let input_bytes = fs::metadata(&input_path).map(|m| m.len() as usize).unwrap_or(0);
    let input_size = format_size(input_bytes);

    let progress_verb = if is_optimize { "Optimizing" } else { "Converting" };
    emit_progress(&app, 0, 1, &format!("{progress_verb}..."));
    let start = std::time::Instant::now();

    let ext = input_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let is_pdf = ext == "pdf";

    let min_res = options.min_resolution;
    let split = !options.no_split;
    if !is_pdf {
        if let Some((src_w, src_h, will_split)) = check_source_quality(&input_path, min_res, split) {
            let reason = if will_split {
                format!("low res after split ({}/2 x {src_h})", src_w)
            } else {
                format!("low res ({src_w}x{src_h})")
            };
            emit_progress(&app, 0, 1, &format!("Skipped: {reason}"));
            return Ok(ConvertResult {
                output_path: input_path.to_string_lossy().to_string(),
                output_size: String::new(),
                input_size,
                input_bytes,
                output_bytes: 0,
                title,
                elapsed: "0.0s".to_string(),
                skipped: true,
                skip_reason: reason,
            });
        }
    }

    let grayscale = if is_optimize { !options.preserve_color } else { true };
    let resize = true;

    if is_optimize && !is_pdf {
        if let Some(reason) = check_optimize_savings(
            &input_path, dev_w, dev_h, quality, options.contrast, grayscale,
        ) {
            emit_progress(&app, 0, 1, &format!("Skipped: {reason}"));
            return Ok(ConvertResult {
                output_path: input_path.to_string_lossy().to_string(),
                output_size: String::new(),
                input_size,
                input_bytes,
                output_bytes: 0,
                title,
                elapsed: "0.0s".to_string(),
                skipped: true,
                skip_reason: reason,
            });
        }
    }

    if cancel.0.load(Ordering::Relaxed) {
        return Err("Cancelled".to_string());
    }

    let output_path = if options.device == "pdf-optimize" {
        if !is_pdf {
            return Err("PDF Optimize only works with PDF files".to_string());
        }
        let pdf_path = expected_output.clone();
        let max_dim = if options.max_image_dim > 0 { options.max_image_dim } else { dev_w };
        optimize_pdf_file(&input_path, &pdf_path, quality, grayscale, max_dim, &cancel.0, &app)
            .map_err(|e| { let _ = fs::remove_file(&pdf_path); e })?;
        pdf_path
    } else if is_cbz_output(&options.device) {
        let cbz_path = expected_output.clone();
        if is_pdf {
            let tmp_dir = tempfile::TempDir::new()
                .map_err(|e| format!("Cannot create temp dir: {e}"))?;
            emit_progress(&app, 0, 1, "Rendering PDF...");
            render_pdf_to_dir(&input_path, tmp_dir.path(), &cancel.0)?;
            optimize_dir_to_cbz(tmp_dir.path(), &cbz_path, dev_w, dev_h, quality, options.contrast, grayscale, resize, !options.no_split, &cancel.0, &app)
                .map_err(|e| { let _ = fs::remove_file(&cbz_path); e })?;
        } else {
            optimize_cbz(&input_path, &cbz_path, dev_w, dev_h, quality, options.contrast, grayscale, resize, !options.no_split, &cancel.0, &app)
                .map_err(|e| { let _ = fs::remove_file(&cbz_path); e })?;
        }
        cbz_path
    } else {
        use kindling::comic::{build_comic_with_options, ComicOptions as KindlingOptions, DeviceProfile};

        let mobi_path = expected_output.clone();
        let profile = DeviceProfile {
            width: dev_w,
            height: dev_h,
            grayscale: true,
            name: dev_name,
        };
        let kindle_options = KindlingOptions {
            jpeg_quality: quality,
            enhance: options.contrast,
            split: !options.no_split,
            crop: 0,
            panel_view: false,
            embed_source: false,
            title_override: Some(title.clone()),
            author_override: None,
            ..KindlingOptions::default()
        };

        let tmp_dir = tempfile::TempDir::new()
            .map_err(|e| format!("Cannot create temp dir: {e}"))?;

        if is_pdf {
            emit_progress(&app, 0, 1, "Rendering PDF...");
            render_pdf_to_dir(&input_path, tmp_dir.path(), &cancel.0)?;
        } else {
            emit_progress(&app, 0, 1, "Extracting archive...");
            let (_, reordered) = extract_archive_to_dir(&input_path, tmp_dir.path())?;
            if reordered {
                emit_progress(&app, 0, 1, "Pages were out of order, reordered");
            }
        }

        let img_count = fs::read_dir(tmp_dir.path())
            .map(|rd| rd.filter_map(|e| {
                let ext = e.ok()?.path().extension()?.to_str()?.to_lowercase();
                ["jpg", "jpeg", "png", "webp", "ppm", "bmp", "gif", "tif", "tiff"].contains(&ext.as_str()).then_some(())
            }).count())
            .unwrap_or(0);
        if img_count == 0 {
            return Err("No images found".to_string());
        }

        if cancel.0.load(Ordering::Relaxed) {
            return Err("Cancelled".to_string());
        }

        #[cfg(unix)]
        let _gag = StderrGuard::new();

        build_comic_with_options(tmp_dir.path(), &mobi_path, &profile, &kindle_options)
            .map_err(|e| {
                let _ = fs::remove_file(&mobi_path);
                format!("Kindling error: {e}")
            })?;

        #[cfg(unix)]
        drop(_gag);

        mobi_path
    };

    cache.0.lock().unwrap_or_else(|p| p.into_inner()).remove(&output_path);

    let output_bytes = fs::metadata(&output_path).map(|m| m.len() as usize).unwrap_or(0);
    let output_size = format_size(output_bytes);

    let duration = start.elapsed();
    let elapsed = if duration.as_secs() >= 60 {
        format!("{}m {:.1}s", duration.as_secs() / 60, duration.as_secs_f64() % 60.0)
    } else {
        format!("{:.1}s", duration.as_secs_f64())
    };

    emit_progress(&app, 1, 1, "Done!");

    Ok(ConvertResult {
        output_path: output_path.to_string_lossy().to_string(),
        output_size,
        input_size,
        input_bytes,
        output_bytes,
        title,
        elapsed,
        skipped: false,
        skip_reason: String::new(),
    })
}

#[cfg(windows)]
fn hide_console_window() {
    use std::sync::Once;

    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        winapi::AllocConsole();
        let hwnd = winapi::GetConsoleWindow();
        if !hwnd.is_null() {
            winapi::ShowWindow(hwnd, 0);
        }
    });
}

#[cfg(windows)]
mod winapi {
    extern "system" {
        pub fn AllocConsole() -> i32;
        pub fn GetConsoleWindow() -> *mut std::ffi::c_void;
        pub fn ShowWindow(hwnd: *mut std::ffi::c_void, cmd: i32) -> i32;
    }
}

fn image_ext_from_bytes(data: &[u8]) -> &'static str {
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        "jpg"
    } else if data.len() >= 4 && data[0..4] == [0x89, 0x50, 0x4E, 0x47] {
        "png"
    } else if data.len() >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        "gif"
    } else if data.len() >= 2 && data[0] == 0x42 && data[1] == 0x4D {
        "bmp"
    } else if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        "webp"
    } else {
        "jpg"
    }
}

#[cfg(unix)]
static STDERR_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

#[cfg(unix)]
struct StderrGuard {
    old_fd: i32,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(unix)]
impl StderrGuard {
    fn new() -> Option<Self> {
        use std::os::unix::io::AsRawFd;
        let lock = STDERR_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let devnull = fs::File::open("/dev/null").ok()?;
        let old_fd = unsafe { libc::dup(2) };
        if old_fd < 0 { return None; }
        let rc = unsafe { libc::dup2(devnull.as_raw_fd(), 2) };
        if rc < 0 {
            unsafe { libc::close(old_fd); }
            return None;
        }
        Some(Self { old_fd, _lock: lock })
    }
}

#[cfg(unix)]
impl Drop for StderrGuard {
    fn drop(&mut self) {
        unsafe {
            libc::dup2(self.old_fd, 2);
            libc::close(self.old_fd);
        }
    }
}

fn emit_progress(app: &AppHandle, current: usize, total: usize, message: &str) {
    let _ = app.emit(
        "convert-progress",
        ConvertProgress {
            current,
            total,
            message: message.to_string(),
        },
    );
}

fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else {
        format!("{:.1} GB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(500), "500 B");
    }

    #[test]
    fn natural_cmp_orders_pages_numerically() {
        let mut v = vec![
            "10.jpg".to_string(),
            "2.jpg".to_string(),
            "1.jpg".to_string(),
            "11.jpg".to_string(),
        ];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["1.jpg", "2.jpg", "10.jpg", "11.jpg"]);
    }

    #[test]
    fn natural_cmp_handles_prefixes_and_padding() {
        let mut v = vec![
            "page-9.png".to_string(),
            "page-10.png".to_string(),
            "page-1.png".to_string(),
        ];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["page-1.png", "page-9.png", "page-10.png"]);
        let mut p = vec!["0002.jpg".to_string(), "0001.jpg".to_string()];
        p.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(p, vec!["0001.jpg", "0002.jpg"]);
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(2048), "2.0 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
    }

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("test_comic.cbz")
    }

    #[test]
    fn kindling_converts_cbz_to_mobi() {
        use kindling::comic::{build_comic_with_options, ComicOptions, DeviceProfile};

        let profile = DeviceProfile {
            width: 600,
            height: 800,
            grayscale: true,
            name: "kindle4",
        };

        let options = ComicOptions {
            jpeg_quality: 20,
            enhance: false,
            split: false,
            crop: 0,
            panel_view: false,
            embed_source: false,
            ..ComicOptions::default()
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let mobi_path = tmp.path().join("test.mobi");

        build_comic_with_options(&fixture_path(), &mobi_path, &profile, &options).unwrap();

        assert!(mobi_path.exists());
        let size = fs::metadata(&mobi_path).unwrap().len();
        assert!(size > 0);
    }

    #[test]
    fn mobi_reader_extracts_images() {
        use kindling::comic::{build_comic_with_options, ComicOptions, DeviceProfile};

        let profile = DeviceProfile {
            width: 600,
            height: 800,
            grayscale: true,
            name: "kindle4",
        };

        let options = ComicOptions {
            jpeg_quality: 20,
            enhance: false,
            split: false,
            crop: 0,
            panel_view: false,
            embed_source: false,
            ..ComicOptions::default()
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let mobi_path = tmp.path().join("test.mobi");
        build_comic_with_options(&fixture_path(), &mobi_path, &profile, &options).unwrap();

        let images = extract_images_from_mobi(&mobi_path).unwrap();
        assert!(images.len() >= 4, "expected at least 4 images, got {}", images.len());
    }

    #[test]
    fn extract_images_from_cbz_returns_nonempty_pages() {
        let images = extract_images_from_cbz(&fixture_path()).unwrap();
        assert!(images.len() >= 4, "expected at least 4 images, got {}", images.len());
        for img in &images {
            assert!(!img.is_empty());
        }
    }

    #[test]
    fn extract_cbz_orders_pages_naturally() {
        use std::io::Write;
        // Scrambled archive order, distinct gray per page; natural sort must
        // yield reading order 1, 2, 10 (not lexical 1, 10, 2).
        let solid_jpeg = |luma: u8| -> Vec<u8> {
            let img = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(
                128,
                128,
                image::Luma([luma]),
            ));
            encode_image(img, 4096, 4096, 90, false, true, false).unwrap()
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let cbz_path = tmp.path().join("order.cbz");
        let file = fs::File::create(&cbz_path).unwrap();
        let mut zip_writer = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        // (name, marker luma) — inserted out of natural order on purpose.
        for (name, luma) in [("10.jpg", 150u8), ("2.jpg", 90u8), ("1.jpg", 30u8)] {
            zip_writer.start_file(name, opts).unwrap();
            zip_writer.write_all(&solid_jpeg(luma)).unwrap();
        }
        zip_writer.finish().unwrap();

        let extracted = extract_images_from_cbz(&cbz_path).unwrap();
        assert_eq!(extracted.len(), 3);

        let markers: Vec<u8> = extracted
            .iter()
            .map(|raw| decode_image(raw).unwrap().to_luma8().get_pixel(64, 64).0[0])
            .collect();
        // Reading order 1(30), 2(90), 10(150) → markers strictly ascending.
        assert!(
            markers[0] < markers[1] && markers[1] < markers[2],
            "pages not in natural order; markers={markers:?}"
        );
        assert!((markers[0] as i16 - 30).abs() < 10, "page 1 marker off: {}", markers[0]);
        assert!((markers[1] as i16 - 90).abs() < 10, "page 2 marker off: {}", markers[1]);
        assert!((markers[2] as i16 - 150).abs() < 10, "page 10 marker off: {}", markers[2]);
    }

    #[test]
    fn split_threshold_matches_aspect_ratio() {
        // process_image_split halves a page only when w > h * 5/4 (1.25:1),
        // the same threshold check_source_quality uses.
        let make = |w: u32, h: u32| -> Vec<u8> {
            let img = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(
                w,
                h,
                image::Luma([128]),
            ));
            encode_image(img, 8192, 8192, 80, false, true, false).unwrap()
        };

        let wide = make(300, 200);
        let pages = process_image_split(&wide, 600, 800, 80, false, true, false, true).unwrap();
        assert_eq!(pages.len(), 2, "1.5:1 page should split");

        let mild = make(220, 200);
        let pages = process_image_split(&mild, 600, 800, 80, false, true, false, true).unwrap();
        assert_eq!(pages.len(), 1, "1.1:1 page should not split");

        let pages = process_image_split(&wide, 600, 800, 80, false, true, false, false).unwrap();
        assert_eq!(pages.len(), 1, "split=false should never split");
    }

    #[test]
    fn image_ext_detection() {
        assert_eq!(image_ext_from_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]), "jpg");
        assert_eq!(image_ext_from_bytes(&[0x89, 0x50, 0x4E, 0x47]), "png");
        assert_eq!(image_ext_from_bytes(b"GIF87a"), "gif");
        assert_eq!(image_ext_from_bytes(b"GIF89a"), "gif");
        assert_eq!(image_ext_from_bytes(&[0x42, 0x4D, 0x00]), "bmp");
        assert_eq!(image_ext_from_bytes(b"GIF"), "jpg"); // too short = fallback
        assert_eq!(image_ext_from_bytes(&[0x00]), "jpg"); // unknown = fallback
    }

    #[test]
    fn device_profile_values() {
        assert_eq!(device_profile("kindle4"), (600, 800, "kindle4"));
        assert_eq!(device_profile("kindle-paperwhite"), (1072, 1448, "kindle_pw"));
        assert_eq!(device_profile("kindle-oasis"), (1264, 1680, "kindle_oasis"));
        assert_eq!(device_profile("kobo-clara-hd"), (1072, 1448, "kobo_clara_hd"));
        assert_eq!(device_profile("optimize"), (2048, 2048, "optimized"));
        assert_eq!(device_profile("pdf-optimize"), (1500, 1500, "pdf_optimized"));
        assert_eq!(device_profile("unknown"), (600, 800, "kindle4"));
    }

    #[test]
    fn is_cbz_output_logic() {
        assert!(is_cbz_output("kobo-clara-hd"));
        assert!(is_cbz_output("optimize"));
        assert!(!is_cbz_output("kindle4"));
        assert!(!is_cbz_output("kindle-paperwhite"));
    }

    #[test]
    fn encode_image_handles_quality_bounds() {
        // Exercises the real encoder at the clamp boundaries (1 and 100),
        // not the std `clamp` fn. A gradient makes quality actually matter.
        let make = || {
            let mut b = image::GrayImage::new(64, 64);
            for (x, y, p) in b.enumerate_pixels_mut() {
                *p = image::Luma([((x * 5 + y * 3) % 256) as u8]);
            }
            image::DynamicImage::ImageLuma8(b)
        };
        let low = encode_image(make(), 64, 64, 1, false, true, false).unwrap();
        let high = encode_image(make(), 64, 64, 100, false, true, false).unwrap();
        assert!(!low.is_empty() && !high.is_empty());
        assert_eq!(image_ext_from_bytes(&low), "jpg");
        assert!(
            high.len() > low.len(),
            "q100 ({}) should exceed q1 ({})",
            high.len(),
            low.len()
        );
    }

    #[test]
    fn mobi_metadata_too_small() {
        let (title, author) = extract_mobi_metadata(&[0u8; 10]);
        assert!(author.is_empty());
        assert!(title.is_empty() || title.chars().all(|c| c == '\0'));
    }

    #[test]
    fn mobi_data_too_small() {
        let result = extract_images_from_mobi_data(&[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn format_size_zero() {
        assert_eq!(format_size(0), "0 B");
    }

    #[test]
    fn format_size_gigabytes() {
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.0 GB");
    }

    #[test]
    fn optimize_skips_already_compact_source() {
        use std::io::Write;
        // Already-q10 pages: re-encoding at the same quality is near-idempotent,
        // so projected savings stay under the 15% skip threshold.
        let tmp = tempfile::TempDir::new().unwrap();
        let cbz_path = tmp.path().join("compact.cbz");
        let file = fs::File::create(&cbz_path).unwrap();
        let mut zip_writer = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        // High-frequency gradient so JPEG yields a non-trivial payload (not a
        // degenerate flat image the encoder could crush further).
        let mut img = image::GrayImage::new(256, 256);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = image::Luma([((x * 7 + y * 13) % 256) as u8]);
        }
        let dynimg = image::DynamicImage::ImageLuma8(img);
        let page = encode_image(dynimg, 2048, 2048, 10, false, true, false).unwrap();

        for i in 0..5 {
            zip_writer.start_file(format!("{i:02}.jpg"), opts).unwrap();
            zip_writer.write_all(&page).unwrap();
        }
        zip_writer.finish().unwrap();

        let result = check_optimize_savings(&cbz_path, 2048, 2048, 10, false, true);
        assert!(
            result.is_some(),
            "already-compact q10 source should skip; got {result:?}"
        );
    }


}
