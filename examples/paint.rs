//! Paint a solid or two-tone JPEG for the end-to-end check.
//!
//! Usage: cargo run --example paint -- <kind> <out.jpg>
//! kinds: glow, blue, forest, night, portrait

use image::{Rgb, RgbImage};

fn main() {
    let mut args = std::env::args().skip(1);
    let kind = args.next().expect("kind");
    let out = args.next().expect("path");
    let mut image = RgbImage::new(64, 64);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = Rgb(match kind.as_str() {
            "glow" => [220, 110, 30],
            "blue" => [30, 80, 210],
            "forest" => [40, 140, 50],
            "night" => [8, 10, 20],
            "portrait" if x > 16 && x < 48 && y > 16 && y < 48 => [210, 150, 120],
            "portrait" => [30, 80, 200],
            other => panic!("unknown kind {other}"),
        });
    }
    image.save(out).expect("save jpeg");
}
