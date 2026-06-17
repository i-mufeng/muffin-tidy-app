mod scanner;
mod livephoto;
mod thumb;
mod export;

use tauri::http::{Request, Response};

// ─────────────────────────── Tauri commands ─────────────────────────────────

#[tauri::command]
fn scan_directory(path: String) -> Result<Vec<scanner::ScannedFile>, String> {
    scanner::scan(std::path::Path::new(&path)).map_err(|e| e.to_string())
}

#[derive(Clone, serde::Serialize)]
struct PreloadProgress {
    done: usize,
    total: usize,
}

/// 扫描完成后受控预热缩略图缓存：仅 2 线程并发（避免瞬时压满 CPU/内存），
/// 每完成一张通过 Channel 上报进度。前端据此显示进度条。
#[tauri::command]
async fn preload_thumbnails(paths: Vec<String>, on_progress: tauri::ipc::Channel<PreloadProgress>) {
    let total = paths.len();
    if total == 0 {
        let _ = on_progress.send(PreloadProgress { done: 0, total: 0 });
        return;
    }
    let _ = tauri::async_runtime::spawn_blocking(move || {
        use rayon::prelude::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let done = AtomicUsize::new(0);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        pool.install(|| {
            paths.par_iter().for_each(|p| {
                let _ = thumb::get_thumb(std::path::Path::new(p));
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                let _ = on_progress.send(PreloadProgress { done: n, total });
            });
        });
    })
    .await;
}

/// Returns a base64-encoded JPEG thumbnail (320px, disk-cached).
/// Handles JPEG/PNG/WebP/GIF/TIFF/BMP + HEIC/RAW via Windows WIC.
#[tauri::command]
fn get_thumbnail(path: String) -> Result<String, String> {
    let bytes = thumb::get_thumb(std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;
    Ok(base64_jpeg(bytes))
}

/// Returns a base64-encoded JPEG for the preview panel (up to 2048px).
#[tauri::command]
fn get_preview(path: String) -> Result<String, String> {
    let bytes = thumb::get_preview(std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;
    Ok(base64_jpeg(bytes))
}

/// 导出（复制整理）筛选保留的媒体文件到目标目录。
/// I/O 密集 → spawn_blocking；进度经 Channel 回传。安全校验在 export::run 内部强制执行。
#[tauri::command]
async fn export_files(
    items: Vec<export::ExportItem>,
    options: export::ExportOptions,
    on_progress: tauri::ipc::Channel<export::ExportProgress>,
) -> Result<export::ExportSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        export::run(&items, &options, &on_progress).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn base64_jpeg(bytes: Vec<u8>) -> String {
    use base64::Engine;
    format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    )
}

// ─────────────────────────── URI scheme: Android Motion Photo video ─────────

fn mphoto_protocol<R: tauri::Runtime>(
    _ctx: tauri::UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let uri = request.uri().to_string();
    let Some(path_str) = decode_query_param(&uri, "path") else {
        return Response::builder().status(400).body(vec![]).unwrap();
    };
    let path = std::path::Path::new(&path_str);

    let Some(offset_from_end) = livephoto::read_motion_photo_offset(path) else {
        return Response::builder().status(404).body(vec![]).unwrap();
    };

    let Ok(meta) = std::fs::metadata(path) else {
        return Response::builder().status(500).body(vec![]).unwrap();
    };
    let video_start = meta.len().saturating_sub(offset_from_end);

    let Ok(mut file) = std::fs::File::open(path) else {
        return Response::builder().status(500).body(vec![]).unwrap();
    };

    use std::io::{Read, Seek, SeekFrom};
    if file.seek(SeekFrom::Start(video_start)).is_err() {
        return Response::builder().status(500).body(vec![]).unwrap();
    }

    let mut video_bytes = Vec::new();
    if file.read_to_end(&mut video_bytes).is_err() {
        return Response::builder().status(500).body(vec![]).unwrap();
    }

    Response::builder()
        .header("Content-Type", "video/mp4")
        .header("Cache-Control", "no-store")
        .body(video_bytes)
        .unwrap()
}

fn decode_query_param(uri: &str, param: &str) -> Option<String> {
    let query = uri.split('?').nth(1)?;
    for part in query.split('&') {
        if let Some(val) = part.strip_prefix(&format!("{}=", param)) {
            return urlencoding::decode(val).ok().map(|s| s.into_owned());
        }
    }
    None
}

// ─────────────────────────── Entry point ────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .register_uri_scheme_protocol("mtidy-mphoto", mphoto_protocol)
        .invoke_handler(tauri::generate_handler![
            scan_directory,
            preload_thumbnails,
            get_thumbnail,
            get_preview,
            export_files,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
