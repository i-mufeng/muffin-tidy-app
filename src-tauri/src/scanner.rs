use std::path::Path;
use anyhow::{bail, Result};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;
use nom_exif::{MediaParser, MediaSource, ExifTag, ExifIter, TrackInfo, TrackInfoTag};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum MediaType { Img, Vdo, Lpo }

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum LiveType { Apple, Android, Huawei }

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScannedFile {
    pub id: String,
    pub source_path: String,
    pub media_type: MediaType,
    pub capture_time: String,
    pub file_size: u64,
    pub live_type: Option<LiveType>,
    pub video_path: Option<String>,
    /// 视频时长（秒），仅视频文件有值
    pub duration: Option<f64>,
    pub exif_info: std::collections::HashMap<String, String>,
}

const IMG_EXTS: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "gif", "bmp", "tif", "tiff",
    "heic", "heif", "webp", "cr2", "cr3", "nef", "nrw", "arw",
    "srf", "sr2", "dng", "orf", "rw2", "raf",
];
const VDO_EXTS: &[&str] = &[
    "mp4", "m4v", "mov", "avi", "mkv", "3gp", "3g2",
    "mpeg", "mpg", "webm", "wmv", "mts", "m2ts",
];

#[derive(Debug, Clone, Serialize)]
pub struct ScanProgress {
    pub stage: &'static str,
    pub current: usize,
    pub total: usize,
}

pub fn scan(
    dir: &Path,
    should_cancel: impl Fn() -> bool,
    mut on_progress: impl FnMut(ScanProgress),
) -> Result<Vec<ScannedFile>> {
    let dir = dunce::canonicalize(dir)?;
    if !dir.is_dir() { bail!("扫描路径不是文件夹"); }
    let mut files: Vec<ScannedFile> = Vec::new();
    let mut paths = Vec::new();
    let mut identifiers = Vec::new();
    let mut last_progress = std::time::Instant::now();
    on_progress(ScanProgress { stage: "discovering", current: 0, total: 0 });

    for entry in WalkDir::new(&dir).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        if should_cancel() {
            bail!("scan cancelled");
        }
        let path = entry.path();
        if path.is_dir() { continue; }

        if entry.path().components().any(|c| {
            c.as_os_str().to_str().map(|s| s.starts_with('.')).unwrap_or(false)
        }) { continue; }

        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        let media_type = if IMG_EXTS.contains(&ext.as_str()) {
            MediaType::Img
        } else if VDO_EXTS.contains(&ext.as_str()) {
            MediaType::Vdo
        } else {
            continue;
        };

        paths.push((path.to_path_buf(), ext, media_type));
        if last_progress.elapsed() >= std::time::Duration::from_millis(100) {
            on_progress(ScanProgress { stage: "discovering", current: paths.len(), total: 0 });
            last_progress = std::time::Instant::now();
        }
    }
    if should_cancel() { bail!("scan cancelled"); }
    let total = paths.len();
    on_progress(ScanProgress { stage: "discovering", current: total, total: 0 });
    on_progress(ScanProgress { stage: "metadata", current: 0, total });
    last_progress = std::time::Instant::now();
    for (index, (path, ext, media_type)) in paths.into_iter().enumerate() {
        if should_cancel() { bail!("scan cancelled"); }
        let path = path.as_path();
        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let metadata = extract_image_metadata(path);
        let capture_time = metadata.capture_time;
        let path_str = path.to_string_lossy().to_string();
        let id = hash_str(&path_str);

        let duration = if matches!(media_type, MediaType::Vdo) {
            extract_video_duration(path)
        } else {
            None
        };

        let (is_motion, xmp_id) = if matches!(media_type, MediaType::Img)
            && matches!(ext.as_str(), "jpg" | "jpeg")
        {
            crate::livephoto::read_xmp_data(path)
        } else {
            (false, None)
        };

        let (live_type, media_type_final, video_path) = if is_motion {
            // Android Motion Photo：视频内嵌在 JPG 内，video_path 指向自身
            // （前端用 mtidy-mphoto 协议从同一文件提取视频段）
            (Some(LiveType::Android), MediaType::Lpo, Some(path_str.clone()))
        } else {
            (None, media_type, None)
        };

        let exif_info = metadata.exif_info;
        identifiers.push(metadata.content_identifier.or(xmp_id));

        files.push(ScannedFile {
            id,
            source_path: path_str,
            media_type: media_type_final,
            capture_time,
            file_size,
            live_type,
            video_path,
            duration,
            exif_info,
        });
        if index + 1 == total || last_progress.elapsed() >= std::time::Duration::from_millis(100) {
            on_progress(ScanProgress { stage: "metadata", current: index + 1, total });
            last_progress = std::time::Instant::now();
        }
    }
    if total == 0 { on_progress(ScanProgress { stage: "metadata", current: 0, total }); }
    pair_live_photos(&mut files, identifiers, &should_cancel, &mut on_progress)?;
    Ok(files)
}

fn pair_live_photos(
    files: &mut Vec<ScannedFile>,
    identifiers: Vec<Option<String>>,
    should_cancel: &impl Fn() -> bool,
    on_progress: &mut impl FnMut(ScanProgress),
) -> Result<()> {
    use std::collections::{HashMap, HashSet};

    let total = files.len();
    let mut last_progress = std::time::Instant::now();
    on_progress(ScanProgress { stage: "pairing", current: 0, total });
    if should_cancel() { bail!("scan cancelled"); }

    // 配对后并入图片主条目、需从列表删除的独立视频条目
    let mut to_remove: HashSet<usize> = HashSet::new();
    let mut paired: HashSet<usize> = HashSet::new();

    // 策略 1：ContentIdentifier（iOS Live Photo，EXIF tag 0x9999 / XMP apple-fi）
    let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, identifier) in identifiers.into_iter().enumerate() {
        if should_cancel() { bail!("scan cancelled"); }
        if let Some(id) = identifier {
            by_id.entry(id).or_default().push(i);
        }
        if last_progress.elapsed() >= std::time::Duration::from_millis(100) {
            on_progress(ScanProgress { stage: "pairing", current: i + 1, total });
            last_progress = std::time::Instant::now();
        }
    }
    for indices in by_id.values() {
        if should_cancel() { bail!("scan cancelled"); }
        if indices.len() < 2 { continue; }
        let img_i = indices.iter().find(|&&i| matches!(files[i].media_type, MediaType::Img));
        let vdo_i = indices.iter().find(|&&i| matches!(files[i].media_type, MediaType::Vdo));
        if let (Some(&ii), Some(&vi)) = (img_i, vdo_i) {
            files[ii].media_type = MediaType::Lpo;
            files[ii].live_type = Some(LiveType::Apple);
            files[ii].video_path = Some(files[vi].source_path.clone());
            paired.insert(ii);
            paired.insert(vi);
            to_remove.insert(vi); // 视频并入图片，不再单独成条目
        }
    }

    // 策略 2：同目录同名回退（图片 + 同名 .mov/.mp4）
    let mut by_stem: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        if should_cancel() { bail!("scan cancelled"); }
        if paired.contains(&i) { continue; }
        let p = Path::new(&f.source_path);
        if let (Some(parent), Some(stem)) = (
            p.parent().map(|p| p.to_string_lossy().to_lowercase()),
            p.file_stem().and_then(|s| s.to_str()).map(|s| s.to_lowercase()),
        ) {
            by_stem.entry((parent, stem)).or_default().push(i);
        }
    }
    for indices in by_stem.values() {
        if should_cancel() { bail!("scan cancelled"); }
        if indices.len() < 2 { continue; }
        let img_i = indices.iter().find(|&&i| matches!(files[i].media_type, MediaType::Img));
        let vdo_i = indices.iter().find(|&&i| {
            let ext = Path::new(&files[i].source_path)
                .extension().and_then(|e| e.to_str())
                .map(|e| e.to_lowercase()).unwrap_or_default();
            matches!(ext.as_str(), "mov" | "mp4")
        });
        if let (Some(&ii), Some(&vi)) = (img_i, vdo_i) {
            // .mov 同名多为 iOS Live；.mp4 同名多为华为/其他动态照片
            let vdo_ext = Path::new(&files[vi].source_path)
                .extension().and_then(|e| e.to_str())
                .map(|e| e.to_lowercase()).unwrap_or_default();
            let live = if vdo_ext == "mov" { LiveType::Apple } else { LiveType::Huawei };
            files[ii].media_type = MediaType::Lpo;
            files[ii].live_type = Some(live);
            files[ii].video_path = Some(files[vi].source_path.clone());
            to_remove.insert(vi);
        }
    }

    if should_cancel() { bail!("scan cancelled"); }
    // 一次线性压缩，避免大量 Live Photo 配对时反复搬移整个 Vec。
    let mut index = 0;
    files.retain(|_| {
        let keep = !to_remove.contains(&index);
        index += 1;
        keep
    });
    on_progress(ScanProgress { stage: "pairing", current: total, total });
    Ok(())
}

/// 提取视频时长（秒）。用 nom-exif TrackInfo 的 DurationMs。
fn extract_video_duration(path: &Path) -> Option<f64> {
    let ms = MediaSource::open(path).ok()?;
    let mut parser = MediaParser::new();
    let info: TrackInfo = parser.parse_track(ms).ok()?;
    let v = info.get(TrackInfoTag::DurationMs)?;
    // 用 Display 文本解析前导数值（毫秒），规避不同版本 EntryValue 变体差异
    let cleaned: String = v
        .to_string()
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let ms_val: f64 = cleaned.parse().ok()?;
    if ms_val <= 0.0 {
        return None;
    }
    Some(ms_val / 1000.0)
}

fn file_mtime_str(path: &Path) -> String {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| DateTime::<Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|_| "Unknown".to_string())
}

struct ImageMetadata {
    capture_time: String,
    exif_info: std::collections::HashMap<String, String>,
    content_identifier: Option<String>,
}

/// 一次 EXIF 遍历同时收集时间、展示字段与配对标识；配对阶段不再打开原图。
fn extract_image_metadata(path: &Path) -> ImageMetadata {
    let mut result = ImageMetadata {
        capture_time: file_mtime_str(path),
        exif_info: std::collections::HashMap::new(),
        content_identifier: None,
    };
    let Ok(ms) = MediaSource::open(path) else { return result; };
    let mut parser = MediaParser::new();
    let Ok(iter): Result<ExifIter, _> = parser.parse_exif(ms) else { return result; };
    let mut found_time = false;
    for entry in iter {
        let val = entry.value().map(|v| v.to_string()).unwrap_or_default();
        if !found_time && matches!(entry.tag().tag(), Some(ExifTag::DateTimeOriginal) | Some(ExifTag::CreateDate)) {
            let time = entry.value().and_then(|value| value.as_datetime().map(|time| time.into_naive()))
                .or_else(|| chrono::NaiveDateTime::parse_from_str(val.trim(), "%Y:%m:%d %H:%M:%S").ok());
            if let Some(ndt) = time {
                use chrono::TimeZone;
                if let chrono::LocalResult::Single(dt) = chrono::Local.from_local_datetime(&ndt) {
                    result.capture_time = dt.format("%Y-%m-%d %H:%M:%S").to_string();
                    found_time = true;
                }
            }
        }
        if entry.tag().code() == 0x9999 && result.content_identifier.is_none() && !val.is_empty() {
            result.content_identifier = Some(val.clone());
        }
        let map = &mut result.exif_info;
        match entry.tag().tag() {
            Some(ExifTag::Make)             => { map.insert("品牌".into(), val); }
            Some(ExifTag::Model)            => { map.insert("型号".into(), val); }
            Some(ExifTag::FNumber)          => { map.insert("光圈".into(), format!("f/{val}")); }
            Some(ExifTag::ISOSpeedRatings)  => { map.insert("ISO".into(), val); }
            Some(ExifTag::ExposureTime)     => { map.insert("快门".into(), val); }
            Some(ExifTag::FocalLength)      => { map.insert("焦距".into(), val); }
            _ => {}
        }
    }
    result
}

fn hash_str(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_stops_when_cancelled() {
        let error = scan(Path::new("."), || true, |_| {}).unwrap_err();
        assert_eq!(error.to_string(), "scan cancelled");
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("tidy-scan-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn file(&self, name: &str) { std::fs::write(self.0.join(name), []).unwrap(); }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    #[test]
    fn empty_directory_reports_all_stage_boundaries() {
        let dir = Fixture::new();
        let mut progress = Vec::new();
        assert!(scan(&dir.0, || false, |p| progress.push(p)).unwrap().is_empty());
        for stage in ["discovering", "metadata", "pairing"] {
            assert!(progress.iter().any(|p| p.stage == stage && p.current == 0 && p.total == 0));
        }
        assert_eq!(progress.last().unwrap().stage, "pairing");
    }

    #[test]
    fn batch_scan_reports_totals_and_preserves_unpaired_files() {
        let dir = Fixture::new();
        for i in 0..300 { dir.file(&format!("photo-{i}.jpg")); }
        for i in (0..300).step_by(2) { dir.file(&format!("photo-{i}.mov")); }
        dir.file("notes.txt");
        let mut progress = Vec::new();
        let files = scan(&dir.0, || false, |p| progress.push(p)).unwrap();
        assert_eq!(files.len(), 300);
        assert_eq!(files.iter().filter(|f| matches!(f.media_type, MediaType::Lpo)).count(), 150);
        for stage in ["metadata", "pairing"] {
            let updates: Vec<_> = progress.iter().filter(|p| p.stage == stage).collect();
            assert_eq!(updates.first().unwrap().current, 0);
            assert_eq!(updates.last().unwrap().current, 450);
            assert!(updates.iter().all(|p| p.total == 450));
            assert!(updates.windows(2).all(|p| p[0].current <= p[1].current));
        }
    }

    #[test]
    fn cancellation_is_observed_at_metadata_and_pairing_boundaries() {
        for stage in ["discovering", "metadata", "pairing"] {
            let dir = Fixture::new();
            dir.file("image.jpg");
            let cancelled = std::cell::Cell::new(false);
            let error = scan(&dir.0, || cancelled.get(), |p| {
                if p.stage == stage { cancelled.set(true); }
            }).unwrap_err();
            assert_eq!(error.to_string(), "scan cancelled");
        }
    }

    #[test]
    fn invalid_root_is_an_error() {
        let dir = Fixture::new();
        assert!(scan(&dir.0.join("missing"), || false, |_| {}).is_err());
        dir.file("file.jpg");
        assert!(scan(&dir.0.join("file.jpg"), || false, |_| {}).is_err());
    }
    #[test]
    fn invalid_exif_preserves_mtime_fallback() {
        let dir = Fixture::new();
        dir.file("plain.jpg");
        let path = dir.0.join("plain.jpg");
        let files = scan(&dir.0, || false, |_| {}).unwrap();
        assert_eq!(files[0].capture_time, file_mtime_str(&path));
        assert!(files[0].exif_info.is_empty());
    }

    #[test]
    fn one_exif_pass_extracts_time_camera_and_identifier() {
        let dir = Fixture::new();
        let fields: [(u16, &[u8]); 3] = [
            (0x010f, b"CameraBrand\0"),
            (0x9003, b"2024:05:06 12:34:56\0"),
            (0x9999, b"content-identifier-123\0"),
        ];
        let mut tiff = b"II\x2a\0\x08\0\0\0".to_vec();
        tiff.extend_from_slice(&(fields.len() as u16).to_le_bytes());
        let mut offset = 8 + 2 + fields.len() * 12 + 4;
        for (tag, value) in fields {
            tiff.extend_from_slice(&tag.to_le_bytes());
            tiff.extend_from_slice(&2u16.to_le_bytes()); // ASCII
            tiff.extend_from_slice(&(value.len() as u32).to_le_bytes());
            tiff.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += value.len();
        }
        tiff.extend_from_slice(&0u32.to_le_bytes());
        for (_, value) in fields { tiff.extend_from_slice(value); }
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        jpeg.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
        jpeg.extend_from_slice(b"Exif\0\0");
        jpeg.extend_from_slice(&tiff);
        jpeg.extend_from_slice(&[0xff, 0xd9]);
        let path = dir.0.join("exif.jpg");
        jpeg.resize(4096, 0);
        std::fs::write(&path, jpeg).unwrap();
        let meta = extract_image_metadata(&path);
        assert_eq!(meta.capture_time, "2024-05-06 12:34:56");
        assert_eq!(meta.exif_info.get("品牌").map(String::as_str), Some("CameraBrand"));
        assert_eq!(meta.content_identifier.as_deref(), Some("content-identifier-123"));
    }

    #[test]
    fn cached_identifiers_pair_different_names_without_reopening_files() {
        let make = |path: &str, media_type| ScannedFile {
            id: path.into(), source_path: path.into(), media_type,
            capture_time: "Unknown".into(), file_size: 0,
            live_type: None, video_path: None, duration: None,
            exif_info: Default::default(),
        };
        let mut files = vec![make("/nonexistent/photo.jpg", MediaType::Img), make("/nonexistent/clip.mov", MediaType::Vdo)];
        pair_live_photos(&mut files, vec![Some("live-id".into()), Some("live-id".into())], &|| false, &mut |_| {}).unwrap();
        assert_eq!(files.len(), 1);
        assert!(matches!(files[0].live_type, Some(LiveType::Apple)));
        assert_eq!(files[0].video_path.as_deref(), Some("/nonexistent/clip.mov"));
    }

    /// Metadata-only synthetic workload, not a 100 GiB image decode benchmark.
    /// Run explicitly: cargo test scan_20k_files_and_sparse_100gib -- --ignored --nocapture
    #[test]
    #[ignore = "creates 20,000 files and one sparse 100 GiB media fixture"]
    fn scan_20k_files_and_sparse_100gib() {
        use std::io::Write;
        let dir = Fixture::new();
        for i in 0..20_000 { dir.file(&format!("photo-{i}.jpg")); }
        let sparse_path = dir.0.join("sparse-100gib.jpg");
        let mut sparse = std::fs::File::create(&sparse_path).unwrap();
        sparse.write_all(&[0xff, 0xd8, 0xff, 0xd9]).unwrap();
        sparse.set_len(100 * 1024 * 1024 * 1024).unwrap();
        drop(sparse);
        let start = std::time::Instant::now();
        let mut updates = 0;
        let files = scan(&dir.0, || false, |_| updates += 1).unwrap();
        assert_eq!(files.len(), 20_001);
        assert_eq!(files.iter().map(|f| f.file_size).sum::<u64>(), 100 * 1024 * 1024 * 1024);
        assert!(updates >= 6);
        println!("metadata-only: {} files, 100 GiB sparse logical size, {:?}, {} progress updates", files.len(), start.elapsed(), updates);
    }

}
