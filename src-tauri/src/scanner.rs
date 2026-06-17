use std::path::Path;
use anyhow::Result;
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

pub fn scan(dir: &Path) -> Result<Vec<ScannedFile>> {
    let dir = dunce::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut files: Vec<ScannedFile> = Vec::new();

    for entry in WalkDir::new(&dir).follow_links(false).into_iter().filter_map(|e| e.ok()) {
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

        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let capture_time = extract_time(path);
        let path_str = path.to_string_lossy().to_string();
        let id = hash_str(&path_str);

        let duration = if matches!(media_type, MediaType::Vdo) {
            extract_video_duration(path)
        } else {
            None
        };

        let (is_motion, _xmp_id) = if matches!(media_type, MediaType::Img)
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

        let exif_info = extract_exif_info(path);

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
    }

    pair_live_photos(&mut files);
    Ok(files)
}

fn pair_live_photos(files: &mut Vec<ScannedFile>) {
    use std::collections::{HashMap, HashSet};

    // 配对后并入图片主条目、需从列表删除的独立视频条目
    let mut to_remove: HashSet<usize> = HashSet::new();
    let mut paired: HashSet<usize> = HashSet::new();

    // 策略 1：ContentIdentifier（iOS Live Photo，EXIF tag 0x9999 / XMP apple-fi）
    let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        if let Some(id) = crate::livephoto::read_content_identifier(Path::new(&f.source_path)) {
            by_id.entry(id).or_default().push(i);
        }
    }
    for indices in by_id.values() {
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

    // 从后往前删除已并入的视频条目，保持其余索引有效
    let mut remove_sorted: Vec<usize> = to_remove.into_iter().collect();
    remove_sorted.sort_unstable_by(|a, b| b.cmp(a));
    for idx in remove_sorted {
        files.remove(idx);
    }
}

fn extract_time(path: &Path) -> String {
    let Ok(ms) = MediaSource::file_path(path) else {
        return file_mtime_str(path);
    };
    let mut parser = MediaParser::new();

    let iter: ExifIter = match parser.parse(ms) {
        Ok(it) => it,
        Err(_) => return file_mtime_str(path),
    };

    for entry in iter {
        if matches!(entry.tag(), Some(ExifTag::DateTimeOriginal) | Some(ExifTag::CreateDate)) {
            if let Some(v) = entry.get_value() {
                let s = v.to_string();
                if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y:%m:%d %H:%M:%S") {
                    use chrono::TimeZone;
                    if let chrono::LocalResult::Single(dt) = chrono::Local.from_local_datetime(&ndt) {
                        return dt.format("%Y-%m-%d %H:%M:%S").to_string();
                    }
                }
            }
        }
    }
    file_mtime_str(path)
}

/// 提取视频时长（秒）。用 nom-exif TrackInfo 的 DurationMs。
fn extract_video_duration(path: &Path) -> Option<f64> {
    let ms = MediaSource::file_path(path).ok()?;
    let mut parser = MediaParser::new();
    let info: TrackInfo = parser.parse(ms).ok()?;
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

fn extract_exif_info(path: &Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(ms) = MediaSource::file_path(path) else { return map; };
    let mut parser = MediaParser::new();
    let Ok(iter): Result<ExifIter, _> = parser.parse(ms) else { return map; };

    for entry in iter {
        let val = entry.get_value().map(|v| v.to_string()).unwrap_or_default();
        match entry.tag() {
            Some(ExifTag::Make)             => { map.insert("品牌".into(), val); }
            Some(ExifTag::Model)            => { map.insert("型号".into(), val); }
            Some(ExifTag::FNumber)          => { map.insert("光圈".into(), format!("f/{val}")); }
            Some(ExifTag::ISOSpeedRatings)  => { map.insert("ISO".into(), val); }
            Some(ExifTag::ExposureTime)     => { map.insert("快门".into(), val); }
            Some(ExifTag::FocalLength)      => { map.insert("焦距".into(), val); }
            _ => {}
        }
    }
    map
}

fn hash_str(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
