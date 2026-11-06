//! Loading scanned pages from PDF.
//!
//! Scanners and scanning apps often save to PDF: each page then contains
//! one large image (usually JPEG = `DCTDecode`, or losslessly compressed
//! `FlateDecode`). We do not render text or vector graphics – from each
//! page we take the largest embedded image.

use std::path::Path;

use lopdf::Document;

use crate::{Gray, ScanError};

/// Loads all PDF pages as grayscale images.
pub fn load_pdf(path: &Path) -> Result<Vec<Gray>, ScanError> {
    let doc = Document::load(path).map_err(|e| ScanError::Pdf(e.to_string()))?;
    let mut out = Vec::new();
    for (num, page_id) in doc.get_pages() {
        let images = doc
            .get_page_images(page_id)
            .map_err(|e| ScanError::Pdf(e.to_string()))?;
        // The largest image on the page is the scan (smaller ones may be logos etc.).
        let Some(img) = images.iter().max_by_key(|i| i.width * i.height) else {
            return Err(ScanError::Pdf(format!("page {num} contains no image")));
        };
        out.push(convert(&doc, img).map_err(|e| ScanError::Pdf(format!("page {num}: {e}")))?);
    }
    Ok(out)
}

/// Converts a PDF image to [`Gray`].
fn convert(doc: &Document, img: &lopdf::xobject::PdfImage) -> Result<Gray, String> {
    let filters = img.filters.clone().unwrap_or_default();
    let has = |f: &str| filters.iter().any(|x| x == f);

    // JPEG – decoded with the `image` crate.
    if has("DCTDecode") {
        let decoded = image::load_from_memory_with_format(img.content, image::ImageFormat::Jpeg)
            .map_err(|e| e.to_string())?;
        return Ok(Gray::from(&decoded.to_luma8()));
    }
    for unsupported in ["JPXDecode", "JBIG2Decode", "CCITTFaxDecode"] {
        if has(unsupported) {
            return Err(format!(
                "image format {unsupported} is not supported – scan to PNG/TIFF or to a grayscale PDF"
            ));
        }
    }

    // Uncompressed or Flate data – let lopdf decompress it.
    let data = if filters.is_empty() {
        img.content.to_vec()
    } else {
        doc.get_object(img.id)
            .and_then(|o| o.as_stream())
            .and_then(|s| s.decompressed_content())
            .map_err(|e| e.to_string())?
    };

    let (w, h) = (img.width as u32, img.height as u32);
    let bpc = img.bits_per_component.unwrap_or(8);
    let image_mask = matches!(
        img.origin_dict.get(b"ImageMask").and_then(|o| o.as_bool()),
        Ok(true)
    );
    let channels = match img.color_space.as_deref() {
        _ if image_mask => 1,
        Some("DeviceGray") | Some("CalGray") | None => 1,
        Some("DeviceRGB") | Some("CalRGB") => 3,
        Some(other) => return Err(format!("unsupported colour space {other}")),
    };
    // A /Decode [1 0] array means inverted values.
    let inverted = img
        .origin_dict
        .get(b"Decode")
        .and_then(|o| o.as_array())
        .ok()
        .and_then(|a| a.first().and_then(|v| v.as_float().ok()))
        .is_some_and(|first| first > 0.5);

    let mut gray = Gray::new(w, h, 255);
    match (bpc, channels) {
        (1, 1) => {
            // 1 bit per pixel, rows padded to bytes, 1 = white.
            let stride = (w as usize).div_ceil(8);
            check_len(&data, stride * h as usize)?;
            for y in 0..h {
                for x in 0..w {
                    let byte = data[y as usize * stride + x as usize / 8];
                    let white = (byte >> (7 - x % 8)) & 1 == 1;
                    gray.set(x, y, if white { 255 } else { 0 });
                }
            }
        }
        (8, 1) => {
            check_len(&data, (w * h) as usize)?;
            gray.data.copy_from_slice(&data[..(w * h) as usize]);
        }
        (8, 3) => {
            check_len(&data, (w * h * 3) as usize)?;
            for (i, &[r, g, b]) in data
                .as_chunks::<3>()
                .0
                .iter()
                .take((w * h) as usize)
                .enumerate()
            {
                // Luma per ITU-R BT.601.
                let l = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
                gray.data[i] = l.round() as u8;
            }
        }
        _ => {
            return Err(format!(
                "unsupported format: {bpc} bits, {channels} channels"
            ));
        }
    }
    if inverted {
        for v in gray.data.iter_mut() {
            *v = 255 - *v;
        }
    }
    Ok(gray)
}

fn check_len(data: &[u8], need: usize) -> Result<(), String> {
    if data.len() < need {
        Err(format!(
            "image has too little data ({} B, expected {need} B)",
            data.len()
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pabaru_render::Bitmap;
    use pabaru_render::pdf::{PdfPage, write_pdf};

    #[test]
    fn loads_own_pdf() {
        let mut bmp = Bitmap::new_white(37, 21);
        bmp.fill_rect(3, 4, 10, 5);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.pdf");
        let pages = [
            PdfPage {
                bitmap: &bmp,
                header_lines: vec!["test".into()],
                margin_mm: 1.0,
            },
            PdfPage {
                bitmap: &bmp,
                header_lines: vec![],
                margin_mm: 1.0,
            },
        ];
        write_pdf(std::fs::File::create(&path).unwrap(), &pages, 600).unwrap();

        let imgs = load_pdf(&path).unwrap();
        assert_eq!(imgs.len(), 2);
        let g = &imgs[0];
        assert_eq!((g.width, g.height), (37, 21));
        for y in 0..21 {
            for x in 0..37 {
                let expected = if bmp.is_black(x, y) { 0 } else { 255 };
                assert_eq!(g.get(x as i64, y as i64), expected);
            }
        }
    }
}
