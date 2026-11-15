//! App icons: Android legacy mipmaps + adaptive icon, and the iOS 1024px icon.

use std::path::Path;

use anyhow::{Context, Result};
use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};

/// (density folder suffix, scale factor relative to mdpi)
const DENSITIES: [(&str, f32); 5] =
    [("mdpi", 1.0), ("hdpi", 1.5), ("xhdpi", 2.0), ("xxhdpi", 3.0), ("xxxhdpi", 4.0)];

const LEGACY_DP: f32 = 48.0;
const ADAPTIVE_DP: f32 = 108.0;

pub struct IconOutput {
    /// Background color of the adaptive icon (ARGB).
    pub adaptive_background: u32,
}

/// Writes `mipmap-*/ic_launcher.png` (and the adaptive foreground) into `res`.
pub fn write_icons(
    res: &Path,
    source: Option<&Path>,
    theme_color: u32,
    icon_background: Option<u32>,
) -> Result<IconOutput> {
    let src = match source {
        Some(path) => Some(
            image::open(path)
                .with_context(|| format!("cannot read icon {}", path.display()))?
                .into_rgba8(),
        ),
        None => None,
    };

    // Fully opaque icons (own background) are scaled to fill the visible
    // mask; transparent logos get padding inside the safe zone.
    let opaque = src.as_ref().is_some_and(is_opaque);
    let adaptive_background = icon_background.unwrap_or_else(|| match &src {
        None => theme_color,
        Some(img) if opaque => to_argb(*img.get_pixel(0, 0)),
        Some(_) => 0xFFFFFFFF,
    });

    for (density, scale) in DENSITIES {
        let dir = res.join(format!("mipmap-{density}"));
        std::fs::create_dir_all(&dir)?;

        let legacy_px = (LEGACY_DP * scale).round() as u32;
        let legacy = match &src {
            Some(img) => fit(img, legacy_px, legacy_px),
            None => default_icon(legacy_px, theme_color, true),
        };
        legacy.save(dir.join("ic_launcher.png"))?;

        let fg_px = (ADAPTIVE_DP * scale).round() as u32;
        let foreground = match &src {
            Some(img) => {
                let content_dp = if opaque { 76.0 } else { 60.0 };
                fit(img, fg_px, (content_dp * scale).round() as u32)
            }
            None => default_icon(fg_px, theme_color, false),
        };
        foreground.save(dir.join("ic_launcher_foreground.png"))?;
    }
    Ok(IconOutput { adaptive_background })
}

/// Writes the single 1024x1024 iOS app icon. iOS masks the corners itself and
/// rejects transparency, so the result is flattened onto an opaque background.
pub fn write_ios_icon(path: &Path, source: Option<&Path>, theme_color: u32, icon_background: Option<u32>) -> Result<()> {
    const SIZE: u32 = 1024;
    let src = match source {
        Some(p) => Some(image::open(p).with_context(|| format!("cannot read icon {}", p.display()))?.into_rgba8()),
        None => None,
    };
    let (background, glyph) = match &src {
        // An opaque icon brings its own background: fill the whole square.
        Some(img) if is_opaque(img) => (icon_background.unwrap_or_else(|| to_argb(*img.get_pixel(0, 0))), fit(img, SIZE, SIZE)),
        Some(img) => (icon_background.unwrap_or(0xFFFFFFFF), fit(img, SIZE, SIZE * 7 / 10)),
        None => {
            // Render the default glyph larger than the canvas and crop the center
            // so the globe fills a sensible share of the icon.
            const BIG: u32 = 1400;
            let big = default_icon(BIG, theme_color, false);
            let off = (BIG - SIZE) / 2;
            (icon_background.unwrap_or(theme_color), imageops::crop_imm(&big, off, off, SIZE, SIZE).to_image())
        }
    };
    let bg = Rgba([(background >> 16) as u8, (background >> 8) as u8, background as u8, 255]);
    let mut out = RgbaImage::from_pixel(SIZE, SIZE, bg);
    imageops::overlay(&mut out, &glyph, 0, 0);
    image::DynamicImage::ImageRgba8(out)
        .to_rgb8()
        .save(path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

fn is_opaque(img: &RgbaImage) -> bool {
    let (w, h) = img.dimensions();
    [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)].iter().all(|&(x, y)| img.get_pixel(x, y)[3] >= 250)
}

fn to_argb(p: Rgba<u8>) -> u32 {
    0xFF00_0000 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32
}

/// Scales `img` to fit in a `content`-sized box centered on a transparent `canvas`.
fn fit(img: &RgbaImage, canvas: u32, content: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let ratio = content as f32 / w.max(h) as f32;
    let nw = ((w as f32 * ratio).round() as u32).max(1);
    let nh = ((h as f32 * ratio).round() as u32).max(1);
    let scaled = imageops::resize(img, nw, nh, FilterType::Lanczos3);
    let mut out = RgbaImage::new(canvas, canvas);
    let x = (canvas as i64 - nw as i64) / 2;
    let y = (canvas as i64 - nh as i64) / 2;
    imageops::overlay(&mut out, &scaled, x, y);
    out
}

/// A simple globe glyph. `legacy` draws it on a theme-colored circle; otherwise
/// it is a white glyph on transparency (the adaptive background supplies color).
fn default_icon(size: u32, theme: u32, legacy: bool) -> RgbaImage {
    let s = size as f32;
    let c = s / 2.0;
    let (globe_r, stroke) = if legacy { (s * 0.27, s * 0.045) } else { (s * 0.20, s * 0.034) };
    let disc_r = s * 0.46;
    let theme_rgb = [(theme >> 16) as u8, (theme >> 8) as u8, theme as u8];
    let half = stroke / 2.0;

    let in_globe = |x: f32, y: f32| -> bool {
        let d = (x * x + y * y).sqrt();
        if (d - globe_r).abs() <= half {
            return true; // outline
        }
        if d > globe_r {
            return false;
        }
        if y.abs() <= half || x.abs() <= half {
            return true; // equator + central meridian
        }
        if (y.abs() - globe_r * 0.5).abs() <= half {
            return true; // two latitude lines
        }
        // meridian ellipse
        let rx = globe_r * 0.5;
        let e = ((x / rx).powi(2) + (y / globe_r).powi(2)).sqrt();
        ((e - 1.0) * rx).abs() <= half
    };

    const SS: u32 = 4; // supersampling for anti-aliasing
    let mut img = RgbaImage::new(size, size);
    for py in 0..size {
        for px in 0..size {
            let (mut glyph, mut disc) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = px as f32 + (sx as f32 + 0.5) / SS as f32 - c;
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32 - c;
                    if in_globe(x, y) {
                        glyph += 1;
                    } else if legacy && (x * x + y * y).sqrt() <= disc_r {
                        disc += 1;
                    }
                }
            }
            let n = (SS * SS) as f32;
            let (g, d) = (glyph as f32 / n, disc as f32 / n);
            let alpha = g + d;
            if alpha <= 0.0 {
                continue;
            }
            let mix = |ch: usize| ((255.0 * g + theme_rgb[ch] as f32 * d) / alpha).round() as u8;
            img.put_pixel(px, py, Rgba([mix(0), mix(1), mix(2), (alpha * 255.0).round() as u8]));
        }
    }
    img
}
