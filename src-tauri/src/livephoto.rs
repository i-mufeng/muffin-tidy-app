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
    read_jpeg_xmp_from(std::fs::File::open(path)?)
}

/// JPEG 段有明确长度；仅读取头部，绝不进入 SOS 后的压缩图像或尾部视频。
/// 512 KiB 是畸形/超长头部的总预算，不再是每张照片固定的读取量。
fn read_jpeg_xmp_from(reader: impl Read) -> anyhow::Result<Option<String>> {
    const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
    let mut reader = reader.take(512 * 1024);
    let mut pair = [0u8; 2];
    if reader.read_exact(&mut pair).is_err() || pair != [0xff, 0xd8] {
        return Ok(None);
    }
    loop {
        if reader.read_exact(&mut pair).is_err() || pair[0] != 0xff {
            return Ok(None);
        }
        // JPEG permits repeated 0xff fill bytes preceding a marker.
        while pair[1] == 0xff {
            if reader.read_exact(&mut pair[1..]).is_err() { return Ok(None); }
        }
        match pair[1] {
            0xda | 0xd9 => return Ok(None), // SOS / EOI
            0x01 | 0xd0..=0xd7 => continue, // standalone markers
            0x00 | 0xd8 => return Ok(None), // invalid outside entropy data
            _ => {}
        }
        let marker = pair[1];
        if reader.read_exact(&mut pair).is_err() { return Ok(None); }
        let length = u16::from_be_bytes(pair) as usize;
        if length < 2 { return Ok(None); }
        let payload_len = length - 2;
        if payload_len as u64 > reader.limit() { return Ok(None); }
        if marker == 0xe1 {
            let mut payload = vec![0; payload_len];
            if reader.read_exact(&mut payload).is_err() { return Ok(None); }
            if payload.starts_with(XMP_HEADER) {
                return Ok(Some(String::from_utf8_lossy(&payload[XMP_HEADER.len()..]).into_owned()));
            }
        } else {
            let copied = std::io::copy(&mut reader.by_ref().take(payload_len as u64), &mut std::io::sink())?;
            if copied != payload_len as u64 { return Ok(None); }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct CountingReader { data: Cursor<Vec<u8>>, bytes: usize }
    impl Read for CountingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let count = self.data.read(buf)?;
            self.bytes += count;
            Ok(count)
        }
    }
    fn segment(jpeg: &mut Vec<u8>, marker: u8, payload: &[u8]) {
        jpeg.extend_from_slice(&[0xff, marker]);
        jpeg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(payload);
    }
    #[test]
    fn jpeg_xmp_stops_at_app1_without_reading_image_payload() {
        let xml = r#"<x:xmpmeta GCamera:MotionPhoto="1" GCamera:MicroVideoOffset="4096" apple-fi:Identifier="live-123"/>"#;
        let mut jpeg = vec![0xff, 0xd8];
        segment(&mut jpeg, 0xe0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        segment(&mut jpeg, 0xe1, b"Exif\0\0");
        let mut xmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
        xmp.extend_from_slice(xml.as_bytes());
        segment(&mut jpeg, 0xe1, &xmp);
        let header_len = jpeg.len();
        jpeg.extend_from_slice(&[0xff, 0xda]);
        jpeg.resize(jpeg.len() + 1024 * 1024, 0xab);
        let mut reader = CountingReader { data: Cursor::new(jpeg), bytes: 0 };
        let actual = read_jpeg_xmp_from(&mut reader).unwrap().unwrap();
        assert_eq!(actual, xml);
        assert_eq!(reader.bytes, header_len);
        assert!(xmp_has_motion_photo(&actual));
        assert_eq!(extract_xmp_attr(&actual, "apple-fi:Identifier").as_deref(), Some("live-123"));
        assert_eq!(extract_xmp_attr(&actual, "GCamera:MicroVideoOffset").as_deref(), Some("4096"));
    }
    #[test]
    fn jpeg_without_xmp_stops_before_entropy_data() {
        let mut reader = CountingReader {
            data: Cursor::new(vec![0xff, 0xd8, 0xff, 0xda, 0, 2, 1, 2, 3]), bytes: 0,
        };
        assert!(read_jpeg_xmp_from(&mut reader).unwrap().is_none());
        assert_eq!(reader.bytes, 4);
    }
    #[test]
    fn corrupt_segments_and_truncated_headers_are_safe() {
        for bytes in [vec![], vec![0xff], vec![0xff, 0xd8, 0xff, 0xe1, 0, 0],
            vec![0xff, 0xd8, 0xff, 0xe1, 0, 1], vec![0xff, 0xd8, 0xff, 0xe1, 0, 99],
            vec![0xff, 0xd8, 0xff, 0xff], vec![0xff, 0xd8, 0xff, 0xd9]] {
            assert!(read_jpeg_xmp_from(Cursor::new(bytes)).unwrap().is_none());
        }
    }
    #[test]
    fn malformed_header_cannot_exceed_read_budget() {
        let mut jpeg = vec![0xff, 0xd8];
        for _ in 0..20 { segment(&mut jpeg, 0xe2, &vec![0; 60_000]); }
        let mut reader = CountingReader { data: Cursor::new(jpeg), bytes: 0 };
        assert!(read_jpeg_xmp_from(&mut reader).unwrap().is_none());
        assert!(reader.bytes <= 512 * 1024);
    }
}
