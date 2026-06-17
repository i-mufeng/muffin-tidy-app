use std::path::{Path, PathBuf};
use anyhow::Result;
use image::DynamicImage;

const THUMB_SIZE: u32 = 320;
const PREVIEW_SIZE: u32 = 2048;

/// Extensions the `image` crate can't decode — need WIC on Windows
const NEEDS_WIC: &[&str] = &[
    "heic", "heif",                                    // Apple
    "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2",  // Canon / Nikon / Sony
    "dng", "orf", "rw2", "raf", "pef", "rwl", "srw",  // Others
];

/// 视频容器 — 用 Windows Shell 缩略图提供程序提取代表帧（与 scanner::VDO_EXTS 一致）
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
        if let Ok(img) = image::open(path) {
            return encode_resized(img, max_size);
        }
    }

    // 3. Windows WIC — HEIC / RAW，及任何已安装编解码器的格式
    #[cfg(windows)]
    {
        if let Ok(img) = decode_with_wic(path) {
            return encode_resized(img, max_size);
        }
    }

    // 4. 兜底：灰色占位，保证网格不出现破损图标
    Ok(placeholder_jpeg(max_size.min(THUMB_SIZE)))
}

/// JPEG DCT 降采样解码：jpeg-decoder 在解码时直接降到 ≥max_size 的最近 1/2ⁿ，
/// 6MB 大图内存占用从 ~100MB 降到 ~1MB。progressive / CMYK 会回退到 image::open。
fn decode_jpeg_scaled(path: &Path, max_size: u32) -> Result<DynamicImage> {
    use jpeg_decoder::{Decoder, PixelFormat};

    let file = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut dec = Decoder::new(file);
    dec.read_info()?;
    let info = dec.info().ok_or_else(|| anyhow::anyhow!("no jpeg info"))?;

    // 请求目标边长；scale 返回实际输出尺寸（按 DCT 取最近的 1/2ⁿ）
    let target = max_size.min(u16::MAX as u32) as u16;
    let (w, h) = dec.scale(target, target)?;
    let data = dec.decode()?;

    let (w, h) = (w as u32, h as u32);
    let img = match info.pixel_format {
        PixelFormat::RGB24 => image::RgbImage::from_raw(w, h, data).map(DynamicImage::ImageRgb8),
        PixelFormat::L8 => image::GrayImage::from_raw(w, h, data).map(DynamicImage::ImageLuma8),
        // L16 / CMYK32 罕见 → 交给 image::open 回退
        _ => None,
    };
    img.ok_or_else(|| anyhow::anyhow!("unsupported jpeg pixel format"))
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

        let mut pixels = vec![0u8; (w * h * 4) as usize];
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
fn decode_with_wic(path: &Path) -> Result<DynamicImage> {
    use windows::{
        core::HSTRING,
        Win32::{
            Graphics::Imaging::{
                CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA,
                IWICFormatConverter, IWICImagingFactory,
                WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
                WICDecodeMetadataCacheOnDemand,
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

        let converter: IWICFormatConverter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;

        let stride = w * 4;
        let mut pixels = vec![0u8; (h * stride) as usize];
        converter.CopyPixels(std::ptr::null(), stride, &mut pixels)?;

        let buf = image::RgbaImage::from_raw(w, h, pixels)
            .ok_or_else(|| anyhow::anyhow!("WIC: buffer size mismatch"))?;
        Ok(DynamicImage::ImageRgba8(buf))
    }
}
