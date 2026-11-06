//! Page export to PNG (1 bit per pixel, with the DPI stored).

use std::io::Write;

use crate::{Bitmap, RenderError};

/// Writes the bitmap as a PNG. The DPI is stored in the `pHYs` chunk so that
/// viewers and print programs print it at the correct size.
pub fn write_png<W: Write>(out: W, bmp: &Bitmap, dpi: u32) -> Result<(), RenderError> {
    let mut enc = ::png::Encoder::new(out, bmp.width, bmp.height);
    enc.set_color(::png::ColorType::Grayscale);
    enc.set_depth(::png::BitDepth::One);
    // PNG stores the resolution in pixels per metre.
    let ppm = (dpi as f64 / 0.0254).round() as u32;
    enc.set_pixel_dims(Some(::png::PixelDimensions {
        xppu: ppm,
        yppu: ppm,
        unit: ::png::Unit::Meter,
    }));
    let mut writer = enc.write_header()?;
    // The bitmap row format is exactly what a 1-bit-deep PNG expects.
    writer.write_image_data(&bmp.data)?;
    writer.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_reads_back() {
        let mut bmp = Bitmap::new_white(17, 5);
        bmp.fill_rect(3, 1, 4, 2);
        let mut buf = Vec::new();
        write_png(&mut buf, &bmp, 600).unwrap();

        let dec = ::png::Decoder::new(std::io::Cursor::new(buf));
        let mut reader = dec.read_info().unwrap();
        let mut img = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut img).unwrap();
        assert_eq!((info.width, info.height), (17, 5));
        assert_eq!(&img[..info.buffer_size()], &bmp.data[..]);
        let dims = reader.info().pixel_dims.unwrap();
        assert_eq!(dims.xppu, 23622); // 600 dpi
    }
}
