//! Generate deterministic fixtures without downloading or reading personal images.
use image::{DynamicImage, ImageBuffer, Rgb, Rgba, RgbaImage};
use std::{fs::File, path::PathBuf};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let folder = PathBuf::from(args.next().unwrap_or_else(|| "artifacts/fixtures".into()));
    let huge = args.any(|a| a == "--huge");
    std::fs::create_dir_all(&folder)?;
    let landscape = ImageBuffer::from_fn(1600, 1000, |x, y| {
        let u = x as f32 / 1600.;
        let v = y as f32 / 1000.;
        let sun = ((u - 0.72).powi(2) + (v - 0.28).powi(2)).sqrt();
        let c = if sun < 0.075 {
            [248, 219, 172]
        } else if v > 0.78 + 0.07 * (u * 8.).sin() {
            [42, 54, 70]
        } else if v > 0.61 + 0.10 * (u * 5. + 1.).sin() {
            [90, 104, 117]
        } else if v > 0.48 + 0.11 * (u * 7. + 3.).sin() {
            [156, 151, 151]
        } else {
            [
                ((205. - v * 60.) as u8),
                ((182. - v * 25.) as u8),
                ((176. + v * 35.) as u8),
            ]
        };
        Rgb(c)
    });
    landscape.save(folder.join("01-dunes.png"))?;
    DynamicImage::ImageRgb8(landscape).save(folder.join("02-dunes.jpg"))?;
    RgbaImage::from_fn(600, 600, |x, y| {
        let dx = x as f32 - 300.;
        let dy = y as f32 - 300.;
        let r = (dx * dx + dy * dy).sqrt();
        Rgba([185, 132, 220, if r < 180. { 170 } else { 0 }])
    })
    .save(folder.join("03-transparency.png"))?;
    let mut gif = image::codecs::gif::GifEncoder::new(File::create(folder.join("04-motion.gif"))?);
    gif.set_repeat(image::codecs::gif::Repeat::Infinite)?;
    for n in 0..24 {
        let frame = RgbaImage::from_fn(240, 160, |x, y| {
            if x.abs_diff(n * 7 + 20) < 18 && y.abs_diff(80) < 18 {
                Rgba([197, 110, 216, 255])
            } else {
                Rgba([24, 27, 40, 255])
            }
        });
        gif.encode_frame(image::Frame::from_parts(
            frame,
            0,
            0,
            image::Delay::from_numer_denom_ms(60, 1),
        ))?;
    }
    std::fs::write(
        folder.join("05-vector.svg"),
        include_bytes!("../assets/flowa.svg"),
    )?;
    std::fs::write(
        folder.join("06-corrupt.png"),
        b"A deliberately invalid image for error testing.",
    )?;
    if huge {
        let huge = ImageBuffer::from_fn(16000, 10000, |x, y| {
            let stripe = ((x / 64 + y / 64) % 2) * 24;
            Rgb([
                ((x * 200 / 16000) + stripe) as u8,
                ((y * 180 / 10000) + stripe) as u8,
                ((x / 500 + y / 500) % 2 * 110 + 60) as u8,
            ])
        });
        huge.save(folder.join("07-huge-160mp.png"))?;
    }
    println!("Fixtures written to {}", folder.display());
    Ok(())
}
