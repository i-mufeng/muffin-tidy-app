use std::path::{Path, PathBuf};
use anyhow::Result;
use image::DynamicImage;

const THUMB_SIZE: u32 = 320;
const PREVIEW_SIZE: u32 = 2048;
// 每个解码任务的工作预算；全局最多两个任务由 lib.rs 控制。
const DECODE_BUDGET: u64 = 256 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 65_535;

fn checked_image_bytes(width: u32, height: u32, bytes_per_pixel: u64) -> Result<usize> {
    anyhow::ensure!(width > 0 && height > 0, "zero-size image");
    let bytes = u64::from(width).checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .ok_or_else(|| anyhow::anyhow!("image dimensions overflow"))?;
    anyhow::ensure!(bytes <= DECODE_BUDGET, "image exceeds decoding budget");
    Ok(usize::try_from(bytes)?)
}

#[cfg(any(windows, test))]
fn scaled_dimensions(width: u32, height: u32, max_size: u32) -> Result<(u32, u32)> {
    anyhow::ensure!(width > 0 && height > 0 && max_size > 0, "zero-size image");
    let longest = width.max(height);
    if longest <= max_size { return Ok((width, height)); }
    Ok((
        (u64::from(width) * u64::from(max_size) / u64::from(longest)).max(1) as u32,
        (u64::from(height) * u64::from(max_size) / u64::from(longest)).max(1) as u32,
    ))
}

fn decode_image_bounded(path: &Path) -> Result<DynamicImage> {
    use image::ImageDecoder;
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(DECODE_BUDGET);
    limits.max_image_width = Some(MAX_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_IMAGE_EDGE);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    checked_image_bytes(width, height, 4)?;
    anyhow::ensure!(decoder.total_bytes() <= DECODE_BUDGET, "image exceeds decoding budget");
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// Extensions the `image` crate can't decode — need WIC on Windows
const NEEDS_WIC: &[&str] = &[
    "heic", "heif",                                    // Apple
    "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2",  // Canon / Nikon / Sony
    "dng", "orf", "rw2", "raf", "pef", "rwl", "srw",  // Others
];

/// 视频容器 — 用 Windows Shell 缩略图提供程序提取代表帧（与 scanner::VDO_EXTS 一致）
#[cfg(windows)]
const VIDEO_EXTS: &[&str] = &[
    "mp4", "m4v", "mov", "avi", "mkv", "3gp", "3g2",
    "mpeg", "mpg", "webm", "wmv", "mts", "m2ts",
];

// ─────────────────────────── public API ────────────────────────────────────

pub fn get_thumb(path: &Path) -> Result<Vec<u8>> {
    get_jpeg_at_size(path, THUMB_SIZE)
}

/// For full-size preview panel.
pub fn get_preview(path: &Path) -> Result<Vec<u8>> {
    get_jpeg_at_size(path, PREVIEW_SIZE)
}

// ─────────────────────────── internals ─────────────────────────────────────

fn get_jpeg_at_size(path: &Path, max_size: u32) -> Result<Vec<u8>> {
    let cache = cache_path_for(path, max_size);
    if let Ok(b) = std::fs::read(&cache) {
        return Ok(b);
    }
    let bytes = generate_jpeg(path, max_size)?;
    if let Some(parent) = cache.parent() {
        let _ = std::fs::create_dir_all(parent);
        let _ = std::fs::write(&cache, &bytes);
    }
    Ok(bytes)
}

fn generate_jpeg(path: &Path, max_size: u32) -> Result<Vec<u8>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    // 0. 视频：Windows Shell 缩略图提供程序提取代表帧（资源管理器同款机制）
    #[cfg(windows)]
    if VIDEO_EXTS.contains(&ext.as_str()) {
        return match decode_video_frame_with_shell(path, max_size) {
            Ok(img) => encode_resized(img, max_size),
            // 系统无对应编解码器 → 灰色占位，不退化成文件类型图标
            Err(_) => Ok(placeholder_jpeg(max_size.min(THUMB_SIZE))),
        };
    }

    // 1. JPEG fast-path: DCT 降采样解码，避免把大图解到全分辨率
    if matches!(ext.as_str(), "jpg" | "jpeg") {
        if let Ok(img) = decode_jpeg_scaled(path, max_size) {
            return encode_resized(img, max_size);
        }
    }

    // 2. image crate 通用解码 (PNG/WebP/GIF/TIFF/BMP，及上面回退的 JPEG)
    if !NEEDS_WIC.contains(&ext.as_str()) {
        if let Ok(img) = decode_image_bounded(path) {
            return encode_resized(img, max_size);
        }
    }

    // 3. Windows WIC — HEIC / RAW，及任何已安装编解码器的格式
    #[cfg(windows)]
    {
        if let Ok(img) = decode_with_wic(path, max_size) {
            return encode_resized(img, max_size);
        }
    }

    // 4. 兜底：灰色占位，保证网格不出现破损图标
    Ok(placeholder_jpeg(max_size.min(THUMB_SIZE)))
}

/// JPEG DCT 降采样解码：jpeg-decoder 在解码时直接降到 ≥max_size 的最近 1/2ⁿ，
/// 输出按 DCT 缩小；渐进 JPEG 仍可能分配原图系数，因此另设原图预算。
fn decode_jpeg_scaled(path: &Path, max_size: u32) -> Result<DynamicImage> {
    use jpeg_decoder::{Decoder, PixelFormat};

    let file = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut dec = Decoder::new(file);
    dec.set_max_decoding_buffer_size(DECODE_BUDGET as usize);
    dec.read_info()?;
    let info = dec.info().ok_or_else(|| anyhow::anyhow!("no jpeg info"))?;
    // RGB 渐进 JPEG 的系数可能达到每像素 6 字节；留在同一任务预算内。
    if info.coding_process != jpeg_decoder::CodingProcess::DctSequential {
        checked_image_bytes(u32::from(info.width), u32::from(info.height), 8)?;
    }

    // 请求目标边长；scale 返回实际输出尺寸（按 DCT 取最近的 1/2ⁿ）
    let target = max_size.min(u16::MAX as u32) as u16;
    let (w, h) = dec.scale(target, target)?;
    let data = dec.decode()?;
    let orientation = dec.exif_data().and_then(image::metadata::Orientation::from_exif_chunk);

    let (w, h) = (w as u32, h as u32);
    let img = match info.pixel_format {
        PixelFormat::RGB24 => image::RgbImage::from_raw(w, h, data).map(DynamicImage::ImageRgb8),
        PixelFormat::L8 => image::GrayImage::from_raw(w, h, data).map(DynamicImage::ImageLuma8),
        // L16 / CMYK32 罕见 → 交给有预算限制的通用解码器回退
        _ => None,
    };
    let mut img = img.ok_or_else(|| anyhow::anyhow!("unsupported jpeg pixel format"))?;
    if let Some(orientation) = orientation { img.apply_orientation(orientation); }
    Ok(img)
}

fn encode_resized(img: DynamicImage, max_size: u32) -> Result<Vec<u8>> {
    // 解码侧已降采样，这里只是精修到目标边长（小图缩放，开销很低）
    let thumb = if img.width() > max_size || img.height() > max_size {
        img.thumbnail(max_size, max_size)
    } else {
        img
    };
    let mut buf = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 82);
    thumb.write_with_encoder(enc)?;
    Ok(buf)
}

fn cache_path_for(path: &Path, max_size: u32) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    // 解码/方向策略变化后不复用旧缓存。
    "bounded-oriented-v2".hash(&mut h);
    // 把源文件 mtime 纳入 key，源图变更后缓存自动失效
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mtime) = meta.modified() {
            mtime.hash(&mut h);
        }
    }
    std::env::temp_dir()
        .join("mtidy-thumbs")
        .join(format!("{:016x}_{}.jpg", h.finish(), max_size))
}

/// 纯灰 JPEG — 所有解码手段都失败时显示
fn placeholder_jpeg(size: u32) -> Vec<u8> {
    let s = size.max(1);
    let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(s, s, image::Rgb([48, 48, 48])));
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Jpeg)
        .unwrap_or_default();
    buf
}

// ─────────────────────────── Windows Shell：视频首帧 ────────────────────────

/// 用 Windows Shell 缩略图提供程序提取视频代表帧（资源管理器同款机制）。
/// 覆盖系统已安装编解码器支持的视频格式；与 WIC 共用同线程 MTA 模型。
#[cfg(windows)]
fn decode_video_frame_with_shell(path: &Path, max_size: u32) -> Result<DynamicImage> {
    use windows::{
        core::HSTRING,
        Win32::{
            Foundation::SIZE,
            Graphics::Gdi::{
                DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC,
                BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
            },
            System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
            UI::Shell::{
                IShellItemImageFactory, SHCreateItemFromParsingName,
                SIIGBF_RESIZETOFIT, SIIGBF_THUMBNAILONLY,
            },
        },
    };

    // 函数任意路径返回都释放 HBITMAP
    struct BmpGuard(HBITMAP);
    impl Drop for BmpGuard {
        fn drop(&mut self) {
            unsafe { let _ = DeleteObject(HGDIOBJ(self.0 .0)); }
        }
    }

    unsafe {
        // 与 WIC 一致用 MTA；S_FALSE = 本线程已初始化，无妨
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let path_str = path.to_str().ok_or_else(|| anyhow::anyhow!("non-UTF8 path"))?;
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path_str), None)?;

        // THUMBNAILONLY：拿不到视频帧就失败，绝不退化成文件类型图标
        let size = SIZE { cx: max_size as i32, cy: max_size as i32 };
        let hbmp = factory.GetImage(size, SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY)?;
        let _guard = BmpGuard(hbmp);

        // 位图真实尺寸（系统按宽高比缩放，未必等于请求值）
        let mut bm = BITMAP::default();
        let got = GetObjectW(
            HGDIOBJ(hbmp.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        anyhow::ensure!(got != 0, "GetObjectW failed");
        let (w, h) = (bm.bmWidth as u32, bm.bmHeight as u32);
        anyhow::ensure!(w > 0 && h > 0, "shell thumbnail zero-size");

        // 32bpp top-down BI_RGB 取像素（内存字节序 BGRA）
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w as i32;
        bmi.bmiHeader.biHeight = -(h as i32); // 负值 = top-down，行序正常
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0 as u32;

        let mut pixels = vec![0u8; checked_image_bytes(w, h, 4)?];
        let hdc = GetDC(None);
        let lines = GetDIBits(
            hdc,
            hbmp,
            0,
            h,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, hdc);
        anyhow::ensure!(lines as u32 == h, "GetDIBits incomplete");

        // BGRA → RGBA，并强制不透明
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }

        let buf = image::RgbaImage::from_raw(w, h, pixels)
            .ok_or_else(|| anyhow::anyhow!("shell frame buffer mismatch"))?;
        Ok(DynamicImage::ImageRgba8(buf))
    }
}

// ─────────────────────────── Windows WIC ───────────────────────────────────

#[cfg(windows)]
fn decode_with_wic(path: &Path, max_size: u32) -> Result<DynamicImage> {
    use windows::{
        core::HSTRING,
        Win32::{
            Graphics::Imaging::{
                CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA,
                IWICFormatConverter, IWICImagingFactory,
                WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
                WICDecodeMetadataCacheOnDemand, WICBitmapInterpolationModeFant,
            },
            System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED},
        },
    };

    unsafe {
        // S_FALSE = already initialized on this thread — that's fine
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;

        let path_str = path.to_str().ok_or_else(|| anyhow::anyhow!("non-UTF8 path"))?;
        let decoder = factory.CreateDecoderFromFilename(
            &HSTRING::from(path_str),
            None,
            windows::Win32::Foundation::GENERIC_ACCESS_RIGHTS(0x8000_0000u32),
            WICDecodeMetadataCacheOnDemand,
        )?;

        let frame = decoder.GetFrame(0)?;

        let mut w = 0u32;
        let mut h = 0u32;
        frame.GetSize(&mut w, &mut h)?;
        anyhow::ensure!(w > 0 && h > 0, "WIC: zero-size frame");

        // 在 CopyPixels 之前缩放 WIC 源，应用层只分配目标尺寸 RGBA。
        let (w, h) = scaled_dimensions(w, h, max_size)?;
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&frame, w, h, WICBitmapInterpolationModeFant)?;
        let converter: IWICFormatConverter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;

        let stride = w.checked_mul(4).ok_or_else(|| anyhow::anyhow!("WIC stride overflow"))?;
        let mut pixels = vec![0u8; checked_image_bytes(w, h, 4)?];
        converter.CopyPixels(std::ptr::null(), stride, &mut pixels)?;

        let buf = image::RgbaImage::from_raw(w, h, pixels)
            .ok_or_else(|| anyhow::anyhow!("WIC: buffer size mismatch"))?;
        Ok(DynamicImage::ImageRgba8(buf))
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn rejects_oversized_allocations_and_zero_dimensions() {
        assert!(checked_image_bytes(100_000, 100_000, 4).is_err());
        assert!(checked_image_bytes(u32::MAX, u32::MAX, 8).is_err());
        assert!(checked_image_bytes(0, 10, 4).is_err());
        assert_eq!(checked_image_bytes(2048, 2048, 4).unwrap(), 16 * 1024 * 1024);
    }

    #[test]
    fn scales_large_frames_before_rgba_allocation() {
        assert_eq!(scaled_dimensions(12_000, 8_000, 320).unwrap(), (320, 213));
        assert_eq!(scaled_dimensions(8_000, 12_000, 2048).unwrap(), (1365, 2048));
        assert_eq!(scaled_dimensions(u32::MAX, 1, 320).unwrap(), (320, 1));
        assert_eq!(scaled_dimensions(20, 10, 320).unwrap(), (20, 10));
        assert!(scaled_dimensions(0, 100, 320).is_err());
    }

    #[test]
    fn bounded_decoder_rejects_large_bmp_header_without_pixel_allocation() {
        // A minimal BMP claiming a huge image; no huge fixture or pixel allocation required.
        let path = std::env::temp_dir().join(format!("tidy-oversize-{}.bmp", std::process::id()));
        let mut bmp = vec![0u8; 54];
        bmp[0..2].copy_from_slice(b"BM");
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&100_000i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&100_000i32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        std::fs::write(&path, bmp).unwrap();
        let result = decode_image_bounded(&path);
        let _ = std::fs::remove_file(path);
        let error = result.unwrap_err();
        assert!(matches!(error.downcast_ref::<image::ImageError>(), Some(image::ImageError::Limits(_)))
            || error.to_string().contains("Image too large"), "{error}");
    }

    #[test]
    fn jpeg_thumbnail_remains_within_requested_dimensions() {
        let path = std::env::temp_dir().join(format!("tidy-scaled-{}.jpg", std::process::id()));
        image::RgbImage::from_pixel(1600, 1000, image::Rgb([80, 120, 160])).save(&path).unwrap();
        let result = generate_jpeg(&path, 320);
        let _ = std::fs::remove_file(path);
        let decoded = image::load_from_memory(&result.unwrap()).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (320, 200));
    }
    #[test]
    fn jpeg_exif_orientation_is_applied_to_preview() {
        let path = std::env::temp_dir().join(format!("tidy-oriented-{}.jpg", std::process::id()));
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(80, 40, image::Rgb([80, 120, 160])))
            .write_to(&mut std::io::Cursor::new(&mut encoded), image::ImageFormat::Jpeg).unwrap();
        // EXIF little-endian TIFF: Orientation = 6 (90 degrees clockwise).
        let exif: &[u8] = &[b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 42, 0, 8, 0, 0, 0,
            1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0];
        let mut jpeg = encoded[..2].to_vec();
        jpeg.extend_from_slice(&[0xff, 0xe1]);
        jpeg.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(exif);
        jpeg.extend_from_slice(&encoded[2..]);
        std::fs::write(&path, jpeg).unwrap();
        let result = generate_jpeg(&path, 2048);
        let _ = std::fs::remove_file(path);
        let decoded = image::load_from_memory(&result.unwrap()).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (40, 80));
    }

}
