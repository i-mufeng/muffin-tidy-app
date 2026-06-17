//! 导出（整理）流程：把筛选保留的媒体文件**复制**到目标目录。
//!
//! 原则性约束（不可违反）：
//!   1. 目标目录不能与源目录相同；
//!   2. 目标目录不能位于源目录内部（绝不写回源路径，避免覆盖/再扫描污染）；
//!   3. 只复制、绝不移动/删除/改写源文件；
//!   4. 字节级复制（std::fs::copy）天然保留全部文件头信息（EXIF/XMP/容器元数据）。
//!
//! 额外用 filetime 把源文件的修改/访问时间一并带到副本上。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use filetime::{set_file_times, FileTime};
use serde::{Deserialize, Serialize};

use crate::scanner::MediaType;

/// 单个时间戳目录下同前缀文件的最大序号，溢出即报错而非静默丢文件。
const MAX_SEQ: u32 = 9999;

// ─────────────────────────── 前后端数据契约 ────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportItem {
    pub source_path: String,
    pub media_type: MediaType,
    /// "YYYY-MM-DD HH:MM:SS"（与 scanner 输出一致），解析失败回退源文件 mtime。
    pub capture_time: String,
    /// 实况照片配对视频。安卓动态照片视频内嵌于 JPG，此值等于 source_path，导出时不重复复制。
    pub video_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub target: String,
    pub source_dir: String,
    /// true=按拍摄日期归档并重命名（YYYY/MM/前缀-时间戳-序号）；false=平铺到目标并保留原文件名。
    pub organize_by_date: bool,
    /// true=内容去重（批内 SHA-256 去重 + 目标已存在相同内容则跳过）。
    pub dedup: bool,
    /// true=只计算计划、不实际写文件。
    pub dry_run: bool,
}

#[derive(Clone, Serialize)]
pub struct ExportProgress {
    pub done: usize,
    pub total: usize,
    pub current: String,
}

#[derive(Default, Serialize)]
pub struct ExportSummary {
    pub exported_img: u32,
    pub exported_vdo: u32,
    pub exported_lpo: u32,
    /// 实际落地的文件数（实况照片成对导出时为 2）。
    pub copied_files: u32,
    pub skipped_dedup: u32,
    pub skipped_conflict: u32,
    pub skipped_error: u32,
    pub errors: Vec<String>,
    /// 本次导出的清单日志路径（写在目标目录；预演模式为 None）。
    pub log_path: Option<String>,
}

// ─────────────────────────── 安全校验（原则性防线）──────────────────────────

/// 校验目标与源目录的隔离关系。任一越界即整体拒绝，绝不开始复制。
pub fn validate_target(target: &Path, source_dir: &Path) -> Result<()> {
    let t = canon(target);
    let s = canon(source_dir);

    if t == s {
        bail!("目标目录不能与源目录相同：{}", t.display());
    }
    if t.starts_with(&s) {
        bail!("目标目录不能位于源目录内部（会覆盖/污染源）：{}", t.display());
    }
    if s.starts_with(&t) {
        bail!("源目录不能位于目标目录内部：{}", s.display());
    }
    Ok(())
}

/// 规范化路径用于比较；不存在时退回原路径（目录选择器返回的都是已存在目录）。
fn canon(p: &Path) -> PathBuf {
    dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

// ─────────────────────────── 主流程 ────────────────────────────────────────

pub fn run(
    items: &[ExportItem],
    opts: &ExportOptions,
    on_progress: &tauri::ipc::Channel<ExportProgress>,
) -> Result<ExportSummary> {
    let target = Path::new(&opts.target);
    let source_dir = Path::new(&opts.source_dir);
    // 原则性防线：校验不过直接整体失败。
    validate_target(target, source_dir)?;

    let started = Local::now();
    let mut summary = ExportSummary::default();
    let mut seen: HashSet<String> = HashSet::new();
    // 逐文件操作记录，结束后写入目标目录的清单日志。
    let mut log: Vec<String> = Vec::new();
    let tag = if opts.dry_run { "预演" } else { "导出" };
    let total = items.len();

    for (i, item) in items.iter().enumerate() {
        let src = Path::new(&item.source_path);
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let _ = on_progress.send(ExportProgress { done: i, total, current: name.clone() });

        // 1. 批内内容去重（按源文件 SHA-256）
        if opts.dedup {
            if let Ok(h) = hash_file(src) {
                if !seen.insert(h) {
                    summary.skipped_dedup += 1;
                    log.push(format!("[重复] {}  (与本次已导出文件内容相同)", item.source_path));
                    continue;
                }
            }
            // 哈希失败不阻断：继续尝试导出该文件。
        }

        let ts = parse_time(item);
        let dir = dest_dir(target, opts.organize_by_date, &ts);
        let img_ext = ext_of(src);

        // 2. 解析图片目标路径（已保证不存在 → 绝不覆盖）
        let img_dst = match resolve_dst(src, &dir, item, &img_ext, opts) {
            Ok(Some(p)) => p,
            Ok(None) => {
                summary.skipped_conflict += 1;
                log.push(format!("[冲突] {}  (目标已存在相同内容)", item.source_path));
                continue;
            }
            Err(e) => {
                summary.skipped_error += 1;
                summary.errors.push(format!("{}：{}", name, e));
                log.push(format!("[失败] {}：{}", item.source_path, e));
                continue;
            }
        };

        // 3. 复制图片
        if !opts.dry_run {
            if let Err(e) = copy_preserving(src, &img_dst) {
                summary.skipped_error += 1;
                summary.errors.push(format!("{}：{}", name, e));
                log.push(format!("[失败] {}  ->  {}：{}", item.source_path, img_dst.display(), e));
                continue;
            }
        }
        log.push(format!("[{}] {}  ->  {}", tag, item.source_path, img_dst.display()));
        let mut copied = 1u32;

        // 4. 实况照片：导出配对视频（安卓内嵌视频 video_path==source_path，无需单独复制）
        if matches!(item.media_type, MediaType::Lpo) {
            if let Some(vp) = &item.video_path {
                if vp != &item.source_path {
                    match export_paired_video(vp, &img_dst, &dir, opts) {
                        Ok(Some(vdst)) => {
                            copied += 1;
                            log.push(format!("[{}] {}  ->  {}", tag, vp, vdst.display()));
                        }
                        Ok(None) => {
                            log.push(format!("[重复] {}  (配对视频，目标已有相同内容)", vp));
                        }
                        Err(e) => {
                            summary.errors.push(format!("{}（视频）：{}", name, e));
                            log.push(format!("[失败] {}（配对视频）：{}", vp, e));
                        }
                    }
                }
            }
        }

        summary.copied_files += copied;
        match item.media_type {
            MediaType::Img => summary.exported_img += 1,
            MediaType::Vdo => summary.exported_vdo += 1,
            MediaType::Lpo => summary.exported_lpo += 1,
        }
    }

    let _ = on_progress.send(ExportProgress { done: total, total, current: String::new() });

    // 写清单日志到目标目录（仅实际导出；预演不落盘）。失败不影响导出结果。
    if !opts.dry_run {
        summary.log_path = write_export_log(target, &started, opts, &summary, &log)
            .map(|p| p.to_string_lossy().into_owned());
    }
    Ok(summary)
}

/// 导出实况照片的配对视频。Ok(Some(dst))=已复制到 dst，Ok(None)=内容重复跳过。
fn export_paired_video(
    video_src: &str,
    img_dst: &Path,
    dir: &Path,
    opts: &ExportOptions,
) -> Result<Option<PathBuf>> {
    let vsrc = Path::new(video_src);
    let vext = ext_of(vsrc);

    // 归档模式下视频与图片共享基名（Lpo-时间戳-序号.mov）；保留名模式用原文件名。
    let preferred = if opts.organize_by_date {
        img_dst.with_extension(&vext)
    } else {
        dir.join(vsrc.file_name().unwrap_or_default())
    };

    let vdst = if !preferred.exists() {
        Some(preferred)
    } else if opts.dedup && files_identical(vsrc, &preferred)? {
        None
    } else {
        // 共享基名被占用且内容不同（罕见）：为视频另取保留原名的空位。
        resolve_keepname(vsrc, dir, opts.dedup)?
    };

    match vdst {
        Some(p) => {
            if !opts.dry_run {
                copy_preserving(vsrc, &p)?;
            }
            Ok(Some(p))
        }
        None => Ok(None),
    }
}

/// 写导出清单日志到目标目录根。返回日志路径；写入失败返回 None（不影响导出）。
fn write_export_log(
    target: &Path,
    started: &DateTime<Local>,
    opts: &ExportOptions,
    summary: &ExportSummary,
    lines: &[String],
) -> Option<PathBuf> {
    let path = target.join(format!(
        "muffin-tidy-export-{}.log",
        started.format("%Y%m%d-%H%M%S")
    ));
    let divider = "─".repeat(60);
    let mut out = String::new();
    out.push_str("# Muffin Tidy 导出日志\n");
    out.push_str(&format!("时间    : {}\n", started.format("%Y-%m-%d %H:%M:%S")));
    out.push_str(&format!("源目录  : {}\n", opts.source_dir));
    out.push_str(&format!("目标目录: {}\n", opts.target));
    out.push_str(&format!(
        "模式    : {} | 智能去重: {}\n",
        if opts.organize_by_date { "按日期归档重命名" } else { "保留原名平铺" },
        if opts.dedup { "开" } else { "关" },
    ));
    out.push_str(&divider);
    out.push('\n');
    for l in lines {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(&divider);
    out.push('\n');
    out.push_str(&format!(
        "成功导出: {} 项 / {} 个文件（图片 {} · 视频 {} · 实况 {}）\n",
        summary.exported_img + summary.exported_vdo + summary.exported_lpo,
        summary.copied_files,
        summary.exported_img,
        summary.exported_vdo,
        summary.exported_lpo,
    ));
    out.push_str(&format!(
        "跳过(重复): {}  跳过(冲突): {}  失败: {}\n",
        summary.skipped_dedup, summary.skipped_conflict, summary.skipped_error,
    ));

    // 目标目录可能尚未创建（极端情况下全部跳过）。
    let _ = std::fs::create_dir_all(target);
    std::fs::write(&path, out).ok().map(|_| path)
}

// ─────────────────────────── 目标路径解析 ──────────────────────────────────

/// 按选项解析目标路径。Ok(None) 表示目标已存在相同内容、应跳过。
fn resolve_dst(
    src: &Path,
    dir: &Path,
    item: &ExportItem,
    ext: &str,
    opts: &ExportOptions,
) -> Result<Option<PathBuf>> {
    if opts.organize_by_date {
        let pfx = prefix(&item.media_type);
        let base = format!("{}-{}", pfx, parse_time(item).format("%Y%m%d%H%M%S"));
        let ext = ext.to_lowercase();
        resolve_generic(src, opts.dedup, |seq| {
            dir.join(format!("{}-{:02}.{}", base, seq, ext))
        })
    } else {
        resolve_keepname(src, dir, opts.dedup)
    }
}

/// 保留原文件名：首选原名，冲突则 `名-02.ext`、`名-03.ext`…
fn resolve_keepname(src: &Path, dir: &Path, dedup: bool) -> Result<Option<PathBuf>> {
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let ext = ext_of(src);
    resolve_generic(src, dedup, |seq| {
        if seq == 1 {
            dir.join(format!("{}.{}", stem, ext))
        } else {
            dir.join(format!("{}-{:02}.{}", stem, seq, ext))
        }
    })
}

/// 通用解析：从 seq=1 起找首个不存在的候选路径。
/// 候选已存在时：若 dedup 且内容相同 → 返回 None（跳过）；否则序号 +1 继续。
fn resolve_generic<F>(src: &Path, dedup: bool, mut candidate: F) -> Result<Option<PathBuf>>
where
    F: FnMut(u32) -> PathBuf,
{
    for seq in 1..=MAX_SEQ {
        let cand = candidate(seq);
        if !cand.exists() {
            return Ok(Some(cand));
        }
        if dedup && files_identical(src, &cand)? {
            return Ok(None);
        }
    }
    bail!("序号溢出：同名候选超过 {} 个", MAX_SEQ)
}

fn dest_dir(target: &Path, organize_by_date: bool, ts: &DateTime<Local>) -> PathBuf {
    if organize_by_date {
        target
            .join(ts.format("%Y").to_string())
            .join(ts.format("%m").to_string())
    } else {
        target.to_path_buf()
    }
}

// ─────────────────────────── 复制 + 元数据保留 ─────────────────────────────

/// 复制并保留时间戳。多重防线确保绝不覆盖、绝不写回源文件。
fn copy_preserving(src: &Path, dst: &Path) -> Result<()> {
    // 终极防线 1：目标与源若解析为同一文件，拒绝（理论上 validate_target 已隔离，双保险）。
    if canon(dst) == canon(src) {
        bail!("目标与源为同一文件，拒绝写入：{}", dst.display());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // 终极防线 2：绝不覆盖已存在文件（路径解析阶段已保证不存在，此处兜底防竞态）。
    if dst.exists() {
        bail!("目标已存在，拒绝覆盖：{}", dst.display());
    }

    std::fs::copy(src, dst)?;

    // 文件内容/头部由字节级复制天然保留；这里再补齐时间戳。
    if let Ok(meta) = std::fs::metadata(src) {
        let mtime = FileTime::from_last_modification_time(&meta);
        let atime = FileTime::from_last_access_time(&meta);
        let _ = set_file_times(dst, atime, mtime);
    }
    Ok(())
}

// ─────────────────────────── 工具函数 ──────────────────────────────────────

fn prefix(t: &MediaType) -> &'static str {
    match t {
        MediaType::Img => "Img",
        MediaType::Vdo => "Vdo",
        MediaType::Lpo => "Lpo",
    }
}

fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .to_lowercase()
}

fn parse_time(item: &ExportItem) -> DateTime<Local> {
    NaiveDateTime::parse_from_str(item.capture_time.trim(), "%Y-%m-%d %H:%M:%S")
        .ok()
        .and_then(|ndt| Local.from_local_datetime(&ndt).single())
        .unwrap_or_else(|| {
            // 回退：源文件修改时间 → 当前时间
            std::fs::metadata(&item.source_path)
                .and_then(|m| m.modified())
                .map(DateTime::<Local>::from)
                .unwrap_or_else(|_| Local::now())
        })
}

fn hash_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn files_identical(a: &Path, b: &Path) -> Result<bool> {
    // 先比大小，不同直接判异，省去哈希开销。
    let (sa, sb) = (std::fs::metadata(a)?.len(), std::fs::metadata(b)?.len());
    if sa != sb {
        return Ok(false);
    }
    Ok(hash_file(a)? == hash_file(b)?)
}

// ─────────────────────────── 单元测试 ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("mtidy-export-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn reject_same_dir() {
        let d = temp_dir("same");
        assert!(validate_target(&d, &d).is_err());
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn reject_target_inside_source() {
        let src = temp_dir("src-outer");
        let dst = src.join("sub");
        fs::create_dir_all(&dst).unwrap();
        assert!(validate_target(&dst, &src).is_err());
        fs::remove_dir_all(&src).ok();
    }

    #[test]
    fn reject_source_inside_target() {
        let tgt = temp_dir("tgt-outer");
        let src = tgt.join("sub");
        fs::create_dir_all(&src).unwrap();
        assert!(validate_target(&tgt, &src).is_err());
        fs::remove_dir_all(&tgt).ok();
    }

    #[test]
    fn accept_disjoint_dirs() {
        let src = temp_dir("disjoint-src");
        let tgt = temp_dir("disjoint-tgt");
        assert!(validate_target(&tgt, &src).is_ok());
        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&tgt).ok();
    }

    #[test]
    fn copy_preserves_bytes_and_refuses_overwrite() {
        let dir = temp_dir("copy");
        let src = dir.join("a.bin");
        fs::write(&src, b"\xFF\xD8header-bytes\x00\x01").unwrap();
        let dst = dir.join("out/a.bin");

        copy_preserving(&src, &dst).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"\xFF\xD8header-bytes\x00\x01");
        // 源文件原样保留
        assert_eq!(fs::read(&src).unwrap(), b"\xFF\xD8header-bytes\x00\x01");
        // 已存在 → 拒绝覆盖
        assert!(copy_preserving(&src, &dst).is_err());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn identical_detection() {
        let dir = temp_dir("ident");
        let a = dir.join("a");
        let b = dir.join("b");
        let c = dir.join("c");
        fs::write(&a, b"same").unwrap();
        fs::write(&b, b"same").unwrap();
        fs::write(&c, b"diff").unwrap();
        assert!(files_identical(&a, &b).unwrap());
        assert!(!files_identical(&a, &c).unwrap());
        fs::remove_dir_all(&dir).ok();
    }
}
