use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{command, AppHandle, Emitter};

const KINDLE4_WIDTH: u32 = 600;
const KINDLE4_HEIGHT: u32 = 800;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConvertOptions {
    pub input_path: String,
    pub output_dir: String,
    pub quality: u8,
    pub contrast: bool,
    pub no_split: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct ConvertProgress {
    pub current: usize,
    pub total: usize,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ConvertResult {
    pub mobi_path: String,
    pub mobi_size: String,
    pub input_size: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
pub struct MobiInfo {
    pub page_count: usize,
    pub file_size: String,
}

#[derive(Debug, Serialize)]
pub struct MobiPage {
    pub image: String,
    pub page: usize,
    pub page_count: usize,
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
    let images = extract_images_from_mobi(&path)?;
    let file_size = fs::metadata(&path)
        .map(|m| format_size(m.len() as usize))
        .unwrap_or_default();

    Ok(MobiInfo {
        page_count: images.len(),
        file_size,
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

#[command]
pub async fn convert_comic(
    app: AppHandle,
    options: ConvertOptions,
) -> Result<ConvertResult, String> {
    use kindling::comic::{build_comic_with_options, ComicOptions as KindlingOptions, DeviceProfile};

    let input_path = PathBuf::from(&options.input_path);
    let output_dir = PathBuf::from(&options.output_dir);

    let title = input_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mobi_path = output_dir.join(format!("{title}.mobi"));

    let profile = DeviceProfile {
        width: KINDLE4_WIDTH,
        height: KINDLE4_HEIGHT,
        grayscale: true,
        name: "kindle4",
    };

    let kindle_options = KindlingOptions {
        jpeg_quality: options.quality,
        enhance: options.contrast,
        split: !options.no_split,
        crop: 0,
        panel_view: false,
        embed_source: false,
        ..KindlingOptions::default()
    };

    emit_progress(&app, 0, 1, "Converting...");

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

    let mobi_size = fs::metadata(&mobi_path)
        .map(|m| format_size(m.len() as usize))
        .unwrap_or_default();

    let input_size = fs::metadata(&input_path)
        .map(|m| format_size(m.len() as usize))
        .unwrap_or_default();

    emit_progress(&app, 1, 1, "Done!");

    Ok(ConvertResult {
        mobi_path: mobi_path.to_string_lossy().to_string(),
        mobi_size,
        input_size,
        title,
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
            width: KINDLE4_WIDTH,
            height: KINDLE4_HEIGHT,
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
            width: KINDLE4_WIDTH,
            height: KINDLE4_HEIGHT,
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
