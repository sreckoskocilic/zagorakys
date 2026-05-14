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
        "optimize" => (9999, 9999, "optimized"),
        _ => (600, 800, "kindle4"),
    }
}

fn is_cbz_output(device: &str) -> bool {
    device.starts_with("kobo") || device == "optimize"
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
    comics.sort();
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

    let record0_offset = if data.len() > 82 {
        u32::from_be_bytes([data[78], data[79], data[80], data[81]]) as usize
    } else {
        return (pdb_title, String::new());
    };

    if record0_offset + 132 > data.len() {
        return (pdb_title, String::new());
    }

    let rec = &data[record0_offset..];

    let full_title = if rec.len() > 92 {
        let title_offset = u32::from_be_bytes([rec[84], rec[85], rec[86], rec[87]]) as usize;
        let title_len = u32::from_be_bytes([rec[88], rec[89], rec[90], rec[91]]) as usize;
        if title_offset + title_len <= rec.len() && title_len > 0 {
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
        let exth_offset = 16 + u32::from_be_bytes([rec[20], rec[21], rec[22], rec[23]]) as usize;
        if exth_offset + 12 <= rec.len() && &rec[exth_offset..exth_offset + 4] == b"EXTH" {
            let num_items = u32::from_be_bytes([
                rec[exth_offset + 8], rec[exth_offset + 9],
                rec[exth_offset + 10], rec[exth_offset + 11],
            ]) as usize;
            let mut pos = exth_offset + 12;
            for _ in 0..num_items {
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
            if lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png") || lower.ends_with(".webp") {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    names.sort();

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
        let map = cache.0.lock().unwrap();
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
    let mut map = cache.0.lock().unwrap();
    if map.len() >= 3 {
        map.clear();
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

fn extract_archive_to_dir(input_path: &Path, dest_dir: &Path) -> Result<usize, String> {
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
                {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        names.sort();

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
        Ok(count)
    } else if archive_type == "rar" {
        #[cfg(windows)]
        hide_console_window();

        let unrar = find_unrar().ok_or_else(|| {
            "unrar not found. Install unrar (brew install unrar / apt install unrar)".to_string()
        })?;

        let status = std::process::Command::new(&unrar)
            .args(["e", "-o+", "-inul", "--"])
            .arg(input_path)
            .arg(dest_dir)
            .status()
            .map_err(|e| format!("Failed to run unrar: {e}"))?;

        if !status.success() {
            let _ = std::process::Command::new(&unrar)
                .args(["e", "-o+", "--"])
                .arg(input_path)
                .arg(dest_dir)
                .status();
        }

        let mut images: Vec<PathBuf> = fs::read_dir(dest_dir)
            .map_err(|e| format!("Cannot read temp dir: {e}"))?
            .filter_map(|e| {
                let path = e.ok()?.path();
                let ext = path.extension()?.to_str()?.to_lowercase();
                if ["jpg", "jpeg", "png", "webp"].contains(&ext.as_str()) {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();
        images.sort();

        let count = images.len();
        for (i, img_path) in images.iter().enumerate() {
            let ext = img_path.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
            let new_name = format!("{:04}.{ext}", i);
            let new_path = dest_dir.join(&new_name);
            if *img_path != new_path {
                let _ = fs::rename(img_path, &new_path);
            }
        }
        Ok(count)
    } else {
        Err(format!("Unsupported archive format: {archive_type}"))
    }
}

fn find_unrar() -> Option<PathBuf> {
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
        if p.exists() {
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

fn render_pdf_to_dir(input_path: &Path, dest_dir: &Path) -> Result<usize, String> {
    #[cfg(windows)]
    hide_console_window();

    if let Some(mutool) = find_tool(&[
        "/opt/homebrew/bin/mutool",
        "/usr/local/bin/mutool",
        "mutool",
    ]) {
        let pattern = dest_dir.join("page_%04d.png");
        let status = std::process::Command::new(&mutool)
            .args(["draw", "-o"])
            .arg(&pattern)
            .args(["-r", "150"])
            .arg(input_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("Failed to run mutool: {e}"))?;
        if status.success() {
            return collect_rendered_images(dest_dir);
        }
    }

    if let Some(pdftoppm) = find_tool(&[
        "/opt/homebrew/bin/pdftoppm",
        "/usr/local/bin/pdftoppm",
        "pdftoppm",
    ]) {
        let prefix = dest_dir.join("page");
        let status = std::process::Command::new(&pdftoppm)
            .args(["-png", "-r", "150"])
            .arg(input_path)
            .arg(&prefix)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("Failed to run pdftoppm: {e}"))?;
        if status.success() {
            return collect_rendered_images(dest_dir);
        }
    }

    Err("PDF renderer not found. Install mupdf-tools (brew install mupdf-tools) or poppler (brew install poppler)".to_string())
}

fn find_tool(candidates: &[&str]) -> Option<PathBuf> {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for &name in candidates {
                if let Some(basename) = Path::new(name).file_name() {
                    let p = dir.join(basename);
                    if p.exists() {
                        return Some(p);
                    }
                    let p = dir.join("resources").join(basename);
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }

    for &name in candidates {
        let p = PathBuf::from(name);
        if p.exists() {
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

    images.sort();
    let count = images.len();
    for (i, img_path) in images.iter().enumerate() {
        let ext = img_path.extension().and_then(|e| e.to_str()).unwrap_or("png");
        let new_path = dir.join(format!("{:04}.{ext}", i));
        if *img_path != new_path {
            let _ = fs::rename(img_path, &new_path);
        }
    }
    Ok(count)
}

fn process_image(
    raw: &[u8],
    width: u32,
    height: u32,
    quality: u8,
    contrast: bool,
    grayscale: bool,
    resize: bool,
) -> Result<Vec<u8>, String> {
    use image::imageops::FilterType;
    use image::ImageReader;
    use std::io::Cursor;

    let img = ImageReader::new(Cursor::new(raw))
        .with_guessed_format()
        .map_err(|e| format!("Image format error: {e}"))?
        .decode()
        .map_err(|e| format!("Decode error: {e}"))?;

    let img = if resize { img.resize(width, height, FilterType::CatmullRom) } else { img };
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

fn optimize_dir_to_cbz(
    dir: &Path,
    output_path: &Path,
    width: u32,
    height: u32,
    quality: u8,
    contrast: bool,
    grayscale: bool,
    resize: bool,
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
            if ["jpg", "jpeg", "png", "webp", "ppm"].contains(&ext.as_str()) {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    images.sort();

    if images.is_empty() {
        return Err("No images found".to_string());
    }

    let total = images.len();
    emit_progress(app, 0, total, &format!("Processing 0/{total}"));

    let raw_images: Vec<Vec<u8>> = images.iter()
        .map(|p| fs::read(p).map_err(|e| format!("Cannot read image: {e}")))
        .collect::<Result<_, _>>()?;

    let processed: Vec<Result<Vec<u8>, String>> = raw_images.par_iter()
        .map(|raw| {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            process_image(raw, width, height, quality, contrast, grayscale, resize)
        })
        .collect();

    let out_file = fs::File::create(output_path).map_err(|e| format!("Cannot create output: {e}"))?;
    let mut zip_writer = zip::ZipWriter::new(out_file);
    let zip_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    for (i, result) in processed.into_iter().enumerate() {
        let jpeg_data = result?;
        emit_progress(app, i + 1, total, &format!("Processing {}/{total}", i + 1));

        let out_name = format!("{:04}.jpg", i);
        zip_writer.start_file(&out_name, zip_options)
            .map_err(|e| format!("ZIP write error: {e}"))?;
        zip_writer.write_all(&jpeg_data)
            .map_err(|e| format!("ZIP write error: {e}"))?;
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

fn optimize_cbz(
    input_path: &Path,
    output_path: &Path,
    width: u32,
    height: u32,
    quality: u8,
    contrast: bool,
    grayscale: bool,
    resize: bool,
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
                if lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png") || lower.ends_with(".webp") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        names.sort();

        for name in &names {
            let mut file = archive.by_name(name).map_err(|e| format!("Cannot read {name}: {e}"))?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).map_err(|e| format!("Read error: {e}"))?;
            entries.push((name.clone(), buf));
        }
    } else if archive_type == "rar" {
        let tmp_dir = tempfile::TempDir::new()
            .map_err(|e| format!("Cannot create temp dir: {e}"))?;
        let count = extract_archive_to_dir(input_path, tmp_dir.path())?;
        if count == 0 {
            return Err("No images found in archive".to_string());
        }
        let mut imgs: Vec<PathBuf> = fs::read_dir(tmp_dir.path())
            .map_err(|e| format!("Cannot read temp dir: {e}"))?
            .filter_map(|e| {
                let path = e.ok()?.path();
                let ext = path.extension()?.to_str()?.to_lowercase();
                if ["jpg", "jpeg", "png", "webp"].contains(&ext.as_str()) {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();
        imgs.sort();
        for img_path in &imgs {
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

    let processed: Vec<Result<(String, Vec<u8>), String>> = entries.par_iter()
        .map(|(name, raw)| {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            let jpeg_data = process_image(raw, width, height, quality, contrast, grayscale, resize)?;
            let out_name = Path::new(name).with_extension("jpg").to_string_lossy().to_string();
            Ok((out_name, jpeg_data))
        })
        .collect();

    let out_file = fs::File::create(output_path).map_err(|e| format!("Cannot create output: {e}"))?;
    let mut zip_writer = zip::ZipWriter::new(out_file);
    let zip_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    for (i, result) in processed.into_iter().enumerate() {
        let (out_name, jpeg_data) = result?;
        emit_progress(app, i + 1, total, &format!("Processing {}/{total}", i + 1));

        zip_writer.start_file(&out_name, zip_options)
            .map_err(|e| format!("ZIP write error: {e}"))?;
        zip_writer.write_all(&jpeg_data)
            .map_err(|e| format!("ZIP write error: {e}"))?;
    }

    zip_writer.finish().map_err(|e| format!("ZIP finalize error: {e}"))?;
    Ok(())
}

#[command]
pub async fn convert_comic(
    app: AppHandle,
    options: ConvertOptions,
    cache: tauri::State<'_, MobiCache>,
    cancel: tauri::State<'_, ConvertCancel>,
) -> Result<ConvertResult, String> {
    cancel.0.store(false, Ordering::Relaxed);
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

    let output_ext = if is_cbz_output(&options.device) { "cbz" } else { "mobi" };
    let base_name = if options.device == "optimize" {
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
        });
    }

    let expected_output = if options.device == "optimize" && expected_output.exists() {
        let mut counter = 1;
        loop {
            let candidate = output_dir.join(format!("{base_name}_{counter}.{output_ext}"));
            if !candidate.exists() {
                break candidate;
            }
            counter += 1;
        }
    } else {
        expected_output
    };

    cache.0.lock().unwrap().remove(&expected_output);

    let is_optimize = options.device == "optimize";
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

    let grayscale = !is_optimize;
    let resize = !is_optimize;
    let output_path = if is_cbz_output(&options.device) {
        let cbz_path = expected_output.clone();
        if is_pdf {
            let tmp_dir = tempfile::TempDir::new()
                .map_err(|e| format!("Cannot create temp dir: {e}"))?;
            emit_progress(&app, 0, 1, "Rendering PDF...");
            render_pdf_to_dir(&input_path, tmp_dir.path())?;
            optimize_dir_to_cbz(tmp_dir.path(), &cbz_path, dev_w, dev_h, quality, options.contrast, grayscale, resize, &cancel.0, &app)?;
        } else {
            optimize_cbz(&input_path, &cbz_path, dev_w, dev_h, quality, options.contrast, grayscale, resize, &cancel.0, &app)?;
        }
        cbz_path
    } else {
        use kindling::comic::{build_comic_with_options, ComicOptions as KindlingOptions, DeviceProfile};

        let mobi_path = output_dir.join(format!("{title}.mobi"));
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
            render_pdf_to_dir(&input_path, tmp_dir.path())?;
        } else {
            emit_progress(&app, 0, 1, "Extracting archive...");
            extract_archive_to_dir(&input_path, tmp_dir.path())?;
        }

        let img_count = fs::read_dir(tmp_dir.path())
            .map(|rd| rd.filter_map(|e| e.ok()).count())
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
            .map_err(|e| format!("Kindling error: {e}"))?;

        #[cfg(unix)]
        drop(_gag);

        mobi_path
    };

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
    } else {
        "jpg"
    }
}

#[cfg(unix)]
struct StderrGuard {
    old_fd: i32,
}

#[cfg(unix)]
impl StderrGuard {
    fn new() -> Option<Self> {
        use std::os::unix::io::AsRawFd;
        let devnull = fs::File::open("/dev/null").ok()?;
        let old_fd = unsafe { libc::dup(2) };
        if old_fd < 0 { return None; }
        unsafe { libc::dup2(devnull.as_raw_fd(), 2) };
        Some(Self { old_fd })
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
    } else {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
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
    fn extract_images_from_cbz_returns_sorted() {
        let images = extract_images_from_cbz(&fixture_path()).unwrap();
        assert!(images.len() >= 4, "expected at least 4 images, got {}", images.len());
        for img in &images {
            assert!(!img.is_empty());
        }
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
        assert_eq!(device_profile("optimize"), (9999, 9999, "optimized"));
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
    fn quality_clamp() {
        assert_eq!(0u8.clamp(1, 100), 1);
        assert_eq!(50u8.clamp(1, 100), 50);
        assert_eq!(100u8.clamp(1, 100), 100);
        assert_eq!(255u8.clamp(1, 100), 100);
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
    fn format_size_large() {
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2048.0 MB");
    }
}
