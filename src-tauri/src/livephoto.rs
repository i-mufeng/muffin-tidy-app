use std::path::Path;
use std::io::Read;

/// Returns (is_android_motion_photo, xmp_content_identifier)
pub fn read_xmp_data(path: &Path) -> (bool, Option<String>) {
    match read_jpeg_xmp(path) {
        Ok(Some(xmp)) => {
            let is_motion = xmp_has_motion_photo(&xmp);
            let id = extract_xmp_attr(&xmp, "apple-fi:Identifier")
                .or_else(|| extract_xmp_attr(&xmp, "Identifier"));
            (is_motion, id)
        }
        _ => (false, None),
    }
}

/// Read iOS ContentIdentifier from EXIF tag 0x9999
pub fn read_content_identifier(path: &Path) -> Option<String> {
    use nom_exif::{MediaParser, MediaSource};
    let ms = MediaSource::file_path(path).ok()?;
    let mut parser = MediaParser::new();
    let iter: nom_exif::ExifIter = parser.parse(ms).ok()?;
    for entry in iter {
        if entry.tag_code() == 0x9999 {
            if let Some(v) = entry.get_value() {
                let s = v.to_string();
                if !s.is_empty() { return Some(s); }
            }
        }
    }
    None
}

/// Returns the byte offset from end-of-file where the embedded MP4 starts.
/// Reads GCamera:MicroVideoOffset from XMP.
pub fn read_motion_photo_offset(path: &Path) -> Option<u64> {
    let xmp = read_jpeg_xmp(path).ok()??;
    // GCamera:MicroVideoOffset or MicroVideo:MicroVideoOffset
    let val = extract_xmp_attr(&xmp, "GCamera:MicroVideoOffset")
        .or_else(|| extract_xmp_attr(&xmp, "MicroVideo:MicroVideoOffset"))?;
    val.parse::<u64>().ok()
}

fn read_jpeg_xmp(path: &Path) -> anyhow::Result<Option<String>> {
    let file = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(65536);
    file.take(512 * 1024).read_to_end(&mut buf)?;

    const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
    let mut i = 0usize;
    while i + 4 < buf.len() {
        if buf[i] == 0xFF && buf[i + 1] == 0xE1 {
            let seg_len = u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as usize;
            let seg_end = i + 2 + seg_len;
            if seg_end > buf.len() { break; }
            let seg_data = &buf[i + 4..seg_end];
            if seg_data.starts_with(XMP_HEADER) {
                return Ok(Some(String::from_utf8_lossy(&seg_data[XMP_HEADER.len()..]).into_owned()));
            }
            i = seg_end;
        } else if buf[i] == 0xFF && buf[i + 1] != 0x00 {
            if i + 4 > buf.len() { break; }
            let seg_len = u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as usize;
            i += 2 + seg_len;
        } else {
            i += 1;
        }
    }
    Ok(None)
}

fn xmp_has_motion_photo(xmp: &str) -> bool {
    let patterns = [
        r#"Camera:MotionPhoto="1""#, r#"Camera:MotionPhoto='1'"#,
        r#"GCamera:MotionPhoto="1""#, r#"GCamera:MotionPhoto='1'"#,
        r#"MicroVideo:MicroVideo="1""#, r#"MicroVideo:MicroVideo='1'"#,
    ];
    patterns.iter().any(|p| xmp.contains(p))
}

fn extract_xmp_attr(xmp: &str, attr: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let search = format!("{}={}", attr, quote);
        if let Some(start) = xmp.find(&search) {
            let rest = &xmp[start + search.len()..];
            if let Some(end) = rest.find(quote) {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}
