//! A minimalist PDF writer.
//!
//! Why our own and not a library? We need very little: on each page a single
//! 1-bit full-page image (at an exact 1:1 scale) and a few lines of text.
//! The custom implementation is about 150 lines, has no dependencies, and we
//! know exactly what gets written to the file.
//!
//! File structure:
//! ```text
//! 1 0 obj Catalog      -> 2 0 obj Pages -> pages
//! 3 0 obj Font (Helvetica, a built-in PDF font)
//! for each page: Page, Contents (drawing commands), Image (bitmap)
//! xref (object offset table), trailer
//! ```

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::{Bitmap, RenderError};

/// A single PDF page.
pub struct PdfPage<'a> {
    /// Full-page bitmap at printer resolution.
    pub bitmap: &'a Bitmap,
    /// Lines of the human-readable header (printed at the top, in the margin).
    pub header_lines: Vec<String>,
    /// Page margin in mm (where the text starts).
    pub margin_mm: f32,
}

/// Header font size in points (1 pt = 1/72 inch).
const FONT_SIZE: f32 = 8.0;
const LEADING: f32 = 10.0;

/// Writes a multi-page PDF. `dpi` is the bitmap (printer) resolution.
pub fn write_pdf<W: Write>(mut out: W, pages: &[PdfPage], dpi: u32) -> Result<(), RenderError> {
    let mut doc = PdfDocument::new(dpi);
    for page in pages {
        doc.add_page(page)?;
    }
    out.write_all(&doc.finish())?;
    Ok(())
}

/// A PDF built incrementally page by page – a page bitmap only needs to be
/// held in memory while it is being added (long backups have hundreds of pages).
///
/// Object numbers: 1 catalog, 2 page tree, 3 font (written only at the end,
/// once the page count is known), from 4 upwards 3 objects per page.
pub struct PdfDocument {
    w: PdfWriter,
    dpi: u32,
    pages: usize,
}

impl PdfDocument {
    pub fn new(dpi: u32) -> Self {
        Self {
            w: PdfWriter::default(),
            dpi,
            pages: 0,
        }
    }

    /// Number of pages added so far.
    pub fn page_count(&self) -> usize {
        self.pages
    }

    /// Adds a page (the image is compressed right away).
    pub fn add_page(&mut self, page: &PdfPage) -> Result<(), RenderError> {
        let (w, dpi) = (&mut self.w, self.dpi);
        let bmp = page.bitmap;
        // Convert pixels to PDF points: 72 points per inch.
        let w_pt = bmp.width as f32 * 72.0 / dpi as f32;
        let h_pt = bmp.height as f32 * 72.0 / dpi as f32;
        let p = 4 + 3 * self.pages;
        let (c, im) = (p + 1, p + 2);
        self.pages += 1;

        w.object(
            p,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w_pt:.3} {h_pt:.3}] \
                 /Resources << /XObject << /Im0 {im} 0 R >> /Font << /F1 3 0 R >> >> \
                 /Contents {c} 0 R >>"
            )
            .as_bytes(),
        );

        // Page content: full-page image + header text.
        let mut content = format!("q {w_pt:.3} 0 0 {h_pt:.3} 0 0 cm /Im0 Do Q\n");
        let margin_pt = page.margin_mm / 25.4 * 72.0;
        for (n, line) in page.header_lines.iter().enumerate() {
            let y = h_pt - margin_pt - FONT_SIZE - n as f32 * LEADING;
            content.push_str(&format!(
                "BT /F1 {FONT_SIZE} Tf {margin_pt:.2} {y:.2} Td ({}) Tj ET\n",
                pdf_text(line)
            ));
        }
        w.stream(c, "", content.as_bytes(), false)?;

        // Image: 1 bit per pixel, DeviceGray (1 = white), Flate-compressed.
        w.stream(
            im,
            &format!(
                "/Type /XObject /Subtype /Image /Width {} /Height {} \
                 /ColorSpace /DeviceGray /BitsPerComponent 1",
                bmp.width, bmp.height
            ),
            &bmp.data,
            true,
        )?;
        Ok(())
    }

    /// Finishes the document and returns the bytes of the PDF file.
    pub fn finish(mut self) -> Vec<u8> {
        let kids: Vec<String> = (0..self.pages)
            .map(|i| format!("{} 0 R", 4 + 3 * i))
            .collect();
        self.w.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
        self.w.object(
            2,
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids.join(" "),
                self.pages
            )
            .as_bytes(),
        );
        self.w.object(
            3,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        );
        self.w.finish()
    }
}

/// Converts text into a safe form for a PDF string: strips (Czech) diacritics
/// (the built-in font lacks some of these characters), replaces other
/// non-ASCII characters with `?` and escapes parentheses and backslashes.
fn pdf_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let mapped = match ch {
            'á' => 'a',
            'č' => 'c',
            'ď' => 'd',
            'é' | 'ě' => 'e',
            'í' => 'i',
            'ň' => 'n',
            'ó' => 'o',
            'ř' => 'r',
            'š' => 's',
            'ť' => 't',
            'ú' | 'ů' => 'u',
            'ý' => 'y',
            'ž' => 'z',
            'Á' => 'A',
            'Č' => 'C',
            'Ď' => 'D',
            'É' | 'Ě' => 'E',
            'Í' => 'I',
            'Ň' => 'N',
            'Ó' => 'O',
            'Ř' => 'R',
            'Š' => 'S',
            'Ť' => 'T',
            'Ú' | 'Ů' => 'U',
            'Ý' => 'Y',
            'Ž' => 'Z',
            c if c.is_ascii() && !c.is_ascii_control() => c,
            _ => '?',
        };
        if matches!(mapped, '(' | ')' | '\\') {
            out.push('\\');
        }
        out.push(mapped);
    }
    out
}

/// Helper that assembles the PDF and remembers object offsets for the xref table.
#[derive(Default)]
struct PdfWriter {
    buf: Vec<u8>,
    /// offsets[n] = offset of object n in the file.
    offsets: Vec<usize>,
}

impl PdfWriter {
    fn ensure_header(&mut self) {
        if self.buf.is_empty() {
            // The second line with bytes > 127 tells programs the file is binary.
            self.buf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        }
    }

    fn record(&mut self, num: usize) {
        self.ensure_header();
        if self.offsets.len() <= num {
            self.offsets.resize(num + 1, 0);
        }
        self.offsets[num] = self.buf.len();
    }

    fn object(&mut self, num: usize, body: &[u8]) {
        self.record(num);
        self.buf
            .extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        self.buf.extend_from_slice(body);
        self.buf.extend_from_slice(b"\nendobj\n");
    }

    fn stream(
        &mut self,
        num: usize,
        dict_extra: &str,
        data: &[u8],
        compress: bool,
    ) -> std::io::Result<()> {
        let (payload, filter) = if compress {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
            enc.write_all(data)?;
            (enc.finish()?, " /Filter /FlateDecode")
        } else {
            (data.to_vec(), "")
        };
        self.record(num);
        self.buf.extend_from_slice(
            format!(
                "{num} 0 obj\n<< {dict_extra}{filter} /Length {} >>\nstream\n",
                payload.len()
            )
            .as_bytes(),
        );
        self.buf.extend_from_slice(&payload);
        self.buf.extend_from_slice(b"\nendstream\nendobj\n");
        Ok(())
    }

    fn finish(mut self) -> Vec<u8> {
        self.ensure_header();
        let xref_pos = self.buf.len();
        let n = self.offsets.len();
        let mut xref = format!("xref\n0 {n}\n0000000000 65535 f \n");
        for off in &self.offsets[1..] {
            xref.push_str(&format!("{off:010} 00000 n \n"));
        }
        xref.push_str(&format!(
            "trailer\n<< /Size {n} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n"
        ));
        self.buf.extend_from_slice(xref.as_bytes());
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    #[test]
    fn pdf_structure_is_valid() {
        let mut bmp = Bitmap::new_white(100, 50);
        bmp.fill_rect(10, 10, 20, 20);
        let pages = [
            PdfPage {
                bitmap: &bmp,
                header_lines: vec!["Záloha (test) 1/2".into()],
                margin_mm: 10.0,
            },
            PdfPage {
                bitmap: &bmp,
                header_lines: vec![],
                margin_mm: 10.0,
            },
        ];
        let mut pdf = Vec::new();
        write_pdf(&mut pdf, &pages, 600).unwrap();

        assert!(pdf.starts_with(b"%PDF-1.4"));
        // startxref must point to the start of the xref table.
        let sx = find(&pdf, b"startxref\n").unwrap() + 10;
        let end = sx + pdf[sx..].iter().position(|&b| b == b'\n').unwrap();
        let xref_pos: usize = std::str::from_utf8(&pdf[sx..end]).unwrap().parse().unwrap();
        assert!(pdf[xref_pos..].starts_with(b"xref\n0 10\n"));

        // Every xref entry points to the start of the right object.
        let table = std::str::from_utf8(&pdf[xref_pos..sx]).unwrap();
        for (num, line) in table.lines().skip(3).take(9).enumerate() {
            let off: usize = line[..10].parse().unwrap();
            let expected = format!("{} 0 obj", num + 1);
            assert!(
                pdf[off..].starts_with(expected.as_bytes()),
                "object {}",
                num + 1
            );
        }

        // Text without diacritics and with escaped parentheses
        // (the header deliberately contains Czech characters).
        assert!(find(&pdf, b"(Zaloha \\(test\\) 1/2) Tj").is_some());

        // The decompressed image matches the bitmap.
        let s = find(&pdf, b"/BitsPerComponent 1 /Filter /FlateDecode").unwrap();
        let data_start = s + find(&pdf[s..], b"stream\n").unwrap() + 7;
        let mut dec = flate2::read::ZlibDecoder::new(&pdf[data_start..]);
        let mut raw = vec![0u8; bmp.data.len()];
        dec.read_exact(&mut raw).unwrap();
        assert_eq!(raw, bmp.data);
    }
}
