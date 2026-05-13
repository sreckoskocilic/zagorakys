use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{command, AppHandle, Emitter};

fn device_profile(device: &str) -> (u32, u32, &'static str) {
    match device {
        "kobo-clara-hd" => (1072, 1448, "kobo_clara_hd"),
        "kindle-paperwhite" => (1072, 1448, "kindle_pw"),
        "kindle-oasis" => (1264, 1680, "kindle_oasis"),
        _ => (600, 800, "kindle4"),
    }
}

fn is_kobo(device: &str) -> bool {
    device.starts_with("kobo")
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConvertOptions {
    pub input_path: String,
    pub output_dir: String,
    pub quality: u8,
    pub contrast: bool,
    pub no_split: bool,
    pub device: String,
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
    pub title: String,
    pub elapsed: String,
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
pub async fn list_comics(dir: String) -> Result<Vec<String>, String> {
    let dir = PathBuf::from(&dir);
    if !dir.is_dir() {
        return Err("Not a directory".to_string());
    }
    let exts = ["cbr", "cbz", "rar", "zip"];
    let mut comics: Vec<String> = fs::read_dir(&dir)
        .map_err(|e| format!("Cannot read directory: {e}"))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
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
                if rec_len < 8 || pos + rec_len > data.len() { break; }
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

        if record.len() > 3 && record[0] == 0xFF && record[1] == 0xD8 && record[2] == 0xFF {
            images.push(record.to_vec());
        }
        if record.len() > 4 && record[0..4] == [0x89, 0x50, 0x4E, 0x47] {
            images.push(record.to_vec());
        }
    }

    Ok(images)
}

#[command]
pub async fn get_mobi_info(path: String) -> Result<MobiInfo, String> {
    let path = PathBuf::from(&path);
    let data = fs::read(&path).map_err(|e| format!("Cannot read MOBI: {e}"))?;
    let (title, author) = extract_mobi_metadata(&data);
    let images = extract_images_from_mobi(&path)?;
    let file_size = format_size(data.len());

    Ok(MobiInfo {
        page_count: images.len(),
        file_size,
        title,
        author,
    })
}

#[command]
pub async fn get_mobi_page(path: String, page: usize) -> Result<MobiPage, String> {
    let path = PathBuf::from(&path);
    let images = extract_images_from_mobi(&path)?;

    if images.is_empty() {
        return Err("No images found in MOBI".to_string());
    }

    let idx = page.min(images.len() - 1);
    let b64 = {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&images[idx]);
        format!("data:image/jpeg;base64,{encoded}")
    };

    Ok(MobiPage {
        image: b64,
        page: idx,
        page_count: images.len(),
    })
}

fn optimize_cbz(
    input_path: &Path,
    output_path: &Path,
    width: u32,
    height: u32,
    quality: u8,
    contrast: bool,
    app: &AppHandle,
) -> Result<(), String> {
    use image::imageops::FilterType;
    use image::ImageReader;
    use std::io::{Cursor, Read, Write};

    let ext = input_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let data = fs::read(input_path).map_err(|e| format!("Cannot read input: {e}"))?;

    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();

    if ext == "cbz" || ext == "zip" {
        let reader = zip::ZipArchive::new(Cursor::new(&data))
            .map_err(|e| format!("Cannot open CBZ: {e}"))?;
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
    } else {
        return Err("CBR extraction not yet supported for Kobo optimize. Use CBZ files.".to_string());
    }

    if entries.is_empty() {
        return Err("No images found in archive".to_string());
    }

    let total = entries.len();
    let out_file = fs::File::create(output_path).map_err(|e| format!("Cannot create output: {e}"))?;
    let mut zip_writer = zip::ZipWriter::new(out_file);
    let zip_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for (i, (name, raw)) in entries.iter().enumerate() {
        emit_progress(app, i, total, &format!("Processing {}/{total}", i + 1));

        let img = ImageReader::new(Cursor::new(raw))
            .with_guessed_format()
            .map_err(|e| format!("Image format error: {e}"))?
            .decode()
            .map_err(|e| format!("Decode error for {name}: {e}"))?;

        let img = img.resize(width, height, FilterType::Lanczos3);

        let img = image::DynamicImage::ImageLuma8(img.to_luma8());

        let img = if contrast {
            image::DynamicImage::ImageLuma8(image::imageops::contrast(&img.to_luma8(), 20.0))
        } else {
            img
        };

        let out_name = Path::new(name).with_extension("jpg").to_string_lossy().to_string();
        let mut jpeg_buf = Cursor::new(Vec::new());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, quality);
        img.write_with_encoder(encoder)
            .map_err(|e| format!("JPEG encode error: {e}"))?;

        zip_writer.start_file(&out_name, zip_options)
            .map_err(|e| format!("ZIP write error: {e}"))?;
        zip_writer.write_all(jpeg_buf.get_ref())
            .map_err(|e| format!("ZIP write error: {e}"))?;
    }

    zip_writer.finish().map_err(|e| format!("ZIP finalize error: {e}"))?;
    Ok(())
}

#[command]
pub async fn convert_comic(
    app: AppHandle,
    options: ConvertOptions,
) -> Result<ConvertResult, String> {
    let input_path = PathBuf::from(&options.input_path);
    let output_dir = PathBuf::from(&options.output_dir);

    let title = input_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let (dev_w, dev_h, dev_name) = device_profile(&options.device);

    let input_size = fs::metadata(&input_path)
        .map(|m| format_size(m.len() as usize))
        .unwrap_or_default();

    emit_progress(&app, 0, 1, "Converting...");
    let start = std::time::Instant::now();

    let output_path = if is_kobo(&options.device) {
        let cbz_path = output_dir.join(format!("{title}.cbz"));
        optimize_cbz(&input_path, &cbz_path, dev_w, dev_h, options.quality, options.contrast, &app)?;
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
            jpeg_quality: options.quality,
            enhance: options.contrast,
            split: !options.no_split,
            crop: 0,
            panel_view: false,
            embed_source: false,
            title_override: Some(title.clone()),
            author_override: None,
            ..KindlingOptions::default()
        };

        #[cfg(windows)]
        ensure_bsdtar_available();

        #[cfg(unix)]
        let _gag = {
            use std::os::unix::io::AsRawFd;
            let devnull = fs::File::open("/dev/null").ok();
            devnull.map(|f| {
                let old = unsafe { libc::dup(2) };
                unsafe { libc::dup2(f.as_raw_fd(), 2) };
                old
            })
        };

        build_comic_with_options(&input_path, &mobi_path, &profile, &kindle_options)
            .map_err(|e| format!("Kindling error: {e}"))?;

        #[cfg(unix)]
        if let Some(old) = _gag {
            unsafe {
                libc::dup2(old, 2);
                libc::close(old);
            }
        }

        mobi_path
    };

    let output_size = fs::metadata(&output_path)
        .map(|m| format_size(m.len() as usize))
        .unwrap_or_default();

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
        title,
        elapsed,
    })
}

#[cfg(windows)]
fn ensure_bsdtar_available() {
    use std::env;
    use std::sync::Once;

    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Copy tar.exe as bsdtar.exe so kindling finds it
        let system32 = PathBuf::from(r"C:\Windows\System32");
        let tar = system32.join("tar.exe");
        if tar.exists() {
            let bin_dir = env::temp_dir().join("zagorakys_bin");
            let _ = fs::create_dir_all(&bin_dir);
            let link = bin_dir.join("bsdtar.exe");
            if !link.exists() {
                let _ = fs::copy(&tar, &link);
            }
            if let Ok(path) = env::var("PATH") {
                env::set_var("PATH", format!("{};{}", bin_dir.display(), path));
            }
        }

        // Hide console so bsdtar spawns don't flash CMD windows
        unsafe {
            winapi::AllocConsole();
            let hwnd = winapi::GetConsoleWindow();
            if !hwnd.is_null() {
                winapi::ShowWindow(hwnd, 0); // SW_HIDE
            }
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
}
