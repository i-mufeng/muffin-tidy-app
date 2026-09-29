mod scanner;
mod livephoto;
mod thumb;
mod export;

use tauri::http::{Request, Response};
use std::sync::atomic::{AtomicU64, Ordering};

// ─────────────────────────── Tauri commands ─────────────────────────────────

static SCAN_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, serde::Serialize)]
struct ScanBatch {
    files: Vec<scanner::ScannedFile>,
    total: usize,
    done: bool,
}

#[tauri::command]
async fn scan_directory(
    path: String,
    on_progress: tauri::ipc::Channel<scanner::ScanProgress>,
    on_files: tauri::ipc::Channel<ScanBatch>,
) -> Result<(), String> {
    let generation = SCAN_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn_blocking(move || {
        let files = scanner::scan(std::path::Path::new(&path), || {
            SCAN_GENERATION.load(Ordering::Relaxed) != generation
        }, |progress| { let _ = on_progress.send(progress); })
        .map_err(|e| e.to_string())?;
        // Bound each JSON payload; never serialize the entire library into one UI message.
        let total = files.len();
        let mut files = files.into_iter();
        loop {
            if SCAN_GENERATION.load(Ordering::Relaxed) != generation {
                return Err("scan cancelled".into());
            }
            let batch = files.by_ref().take(256).collect();
            let done = files.len() == 0;
            on_files.send(ScanBatch { files: batch, total, done }).map_err(|e| e.to_string())?;
            if done { return Ok(()); }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn cancel_scan() {
    SCAN_GENERATION.fetch_add(1, Ordering::SeqCst);
}

#[derive(Clone, serde::Serialize)]
struct PreloadProgress {
    done: usize,
    total: usize,
}

/// 扫描完成后受控预热缩略图缓存：仅 2 线程并发（避免瞬时压满 CPU/内存），
/// 每 100ms 或全部完成时通过 Channel 上报进度，避免缓存命中时刷满 IPC。
#[tauri::command]
async fn preload_thumbnails(paths: Vec<String>, on_progress: tauri::ipc::Channel<PreloadProgress>) {
    let total = paths.len();
    let generation = SCAN_GENERATION.load(Ordering::Relaxed);
    if total == 0 {
        let _ = on_progress.send(PreloadProgress { done: 0, total: 0 });
        return;
    }
    let _ = tauri::async_runtime::spawn_blocking(move || {
        use rayon::prelude::*;
        // 计数和发送共用一把锁，两个工作线程也不会发送倒序进度。
        let progress = std::sync::Mutex::new((0usize, std::time::Instant::now()));
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        pool.install(|| {
            paths.par_iter().for_each(|p| {
                if SCAN_GENERATION.load(Ordering::Relaxed) != generation { return; }
                let permit = ImagePermit::acquire();
                // 等待并发名额期间也可能已经取消或切换了目录。
                if SCAN_GENERATION.load(Ordering::Relaxed) != generation { return; }
                let _ = thumb::get_thumb(std::path::Path::new(p));
                drop(permit);
                if SCAN_GENERATION.load(Ordering::Relaxed) != generation { return; }
                let mut progress = progress.lock().unwrap_or_else(|e| e.into_inner());
                progress.0 += 1;
                if progress.0 == total || progress.1.elapsed() >= std::time::Duration::from_millis(100) {
                    let _ = on_progress.send(PreloadProgress { done: progress.0, total });
                    progress.1 = std::time::Instant::now();
                }
            });
        });
    })
    .await;
}

// 限制大图解码的并发数；等待发生在 blocking 线程，不阻塞窗口事件循环。
static IMAGE_JOBS: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
static IMAGE_READY: std::sync::Condvar = std::sync::Condvar::new();

struct ImagePermit;
impl ImagePermit {
    fn acquire() -> Self {
        let mut active = IMAGE_JOBS.lock().unwrap_or_else(|e| e.into_inner());
        while *active >= 2 {
            active = IMAGE_READY.wait(active).unwrap_or_else(|e| e.into_inner());
        }
        *active += 1;
        Self
    }
}
impl Drop for ImagePermit {
    fn drop(&mut self) {
        let mut active = IMAGE_JOBS.lock().unwrap_or_else(|e| e.into_inner());
        *active -= 1;
        IMAGE_READY.notify_one();
    }
}

#[tauri::command]
async fn get_thumbnail(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = ImagePermit::acquire();
        thumb::get_thumb(std::path::Path::new(&path)).map(base64_jpeg).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_preview(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = ImagePermit::acquire();
        thumb::get_preview(std::path::Path::new(&path)).map(base64_jpeg).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
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
            cancel_scan,
            preload_thumbnails,
            get_thumbnail,
            get_preview,
            export_files,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
