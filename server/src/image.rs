//! Image work the Python version does with Pillow: LANCZOS resize, the
//! coordinate grid and Set-of-Mark overlays, JPEG encoding, base64.
//! Everything but the GDI text labels is plain Rust and builds everywhere.

#[cfg(windows)]
use crate::ffi;
use crate::py::{Error, Res};

/// An RGB image, 3 bytes per pixel.
#[derive(Clone, PartialEq, Debug)]
pub struct Img {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
}

pub type Rgba = (u8, u8, u8, u8);

impl Img {
    pub fn new(w: u32, h: u32) -> Img {
        Img { w, h, rgb: vec![0; (w as usize) * (h as usize) * 3] }
    }

    /// From a top-down 32-bit BGRX buffer (what GDI returns).
    pub fn from_bgrx(w: u32, h: u32, bgrx: &[u8]) -> Img {
        let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
        for px in bgrx.chunks_exact(4).take((w as usize) * (h as usize)) {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        Img { w, h, rgb }
    }

    pub fn to_bgra(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity((self.w as usize) * (self.h as usize) * 4);
        for px in self.rgb.chunks_exact(3) {
            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
        out
    }

    pub fn crop(&self, left: u32, top: u32, right: u32, bottom: u32) -> Img {
        let (w, h) = (right - left, bottom - top);
        let mut out = Img::new(w, h);
        for y in 0..h as usize {
            let src = ((top as usize + y) * self.w as usize + left as usize) * 3;
            let dst = y * w as usize * 3;
            out.rgb[dst..dst + w as usize * 3].copy_from_slice(&self.rgb[src..src + w as usize * 3]);
        }
        out
    }

    /// True when a captured image is uniformly one color (e.g. black DirectX surface).
    pub fn is_blank(&self) -> bool {
        if self.w < 16 || self.h < 16 {
            return false;
        }
        let first = &self.rgb[..3];
        self.rgb.chunks_exact(3).all(|px| px == first)
    }

    /// Alpha-blend one pixel the way Pillow's RGBA drawing does.
    #[inline]
    pub fn blend(&mut self, x: i64, y: i64, c: Rgba) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return;
        }
        let i = (y as usize * self.w as usize + x as usize) * 3;
        let a = c.3 as u32;
        let muldiv = |v: u32, m: u32| -> u32 {
            let t = v * m + 128;
            ((t >> 8) + t) >> 8
        };
        for (k, col) in [c.0, c.1, c.2].into_iter().enumerate() {
            let v = muldiv(self.rgb[i + k] as u32, 255 - a) + muldiv(col as u32, a);
            self.rgb[i + k] = v.min(255) as u8;
        }
    }

    /// `d.rectangle([x0, y0, x1, y1], fill=c)` (inclusive corners)
    pub fn fill_rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: Rgba) {
        for y in y0.max(0)..=y1.min(self.h as i64 - 1) {
            for x in x0.max(0)..=x1.min(self.w as i64 - 1) {
                self.blend(x, y, c);
            }
        }
    }

    /// `d.rectangle([x0, y0, x1, y1], outline=c, width=1)`
    pub fn outline_rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: Rgba) {
        if x1 < x0 || y1 < y0 {
            return;
        }
        self.fill_rect(x0, y0, x1, y0, c);
        if y1 > y0 {
            self.fill_rect(x0, y1, x1, y1, c);
        }
        if y1 - y0 > 1 {
            self.fill_rect(x0, y0 + 1, x0, y1 - 1, c);
            if x1 > x0 {
                self.fill_rect(x1, y0 + 1, x1, y1 - 1, c);
            }
        }
    }

    /// A ring of the given radius and line width around (cx, cy).
    pub fn ring(&mut self, cx: f64, cy: f64, radius: f64, width: f64, c: Rgba) {
        let (lo, hi) = (radius - width, radius);
        let r = radius.ceil() as i64 + 1;
        let (ix, iy) = (cx.round() as i64, cy.round() as i64);
        for y in iy - r..=iy + r {
            for x in ix - r..=ix + r {
                let d = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
                if d > lo && d <= hi {
                    self.blend(x, y, c);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- LANCZOS resize (Pillow's algorithm)

const PRECISION_BITS: u32 = 32 - 8 - 2;

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    let x = x * std::f64::consts::PI;
    x.sin() / x
}

fn lanczos(x: f64) -> f64 {
    if (-3.0..3.0).contains(&x) {
        sinc(x) * sinc(x / 3.0)
    } else {
        0.0
    }
}

/// Per output pixel: first source pixel, and fixed-point weights.
fn coefficients(in_size: usize, out_size: usize) -> Vec<(usize, Vec<i32>)> {
    let scale = in_size as f64 / out_size as f64;
    let filterscale = scale.max(1.0);
    let support = 3.0 * filterscale;
    let ss = 1.0 / filterscale;
    let mut out = Vec::with_capacity(out_size);
    for xx in 0..out_size {
        let center = (xx as f64 + 0.5) * scale;
        let xmin = ((center - support + 0.5) as i64).max(0);
        let xmax = ((center + support + 0.5) as i64).min(in_size as i64);
        let n = (xmax - xmin).max(0) as usize;
        let mut w: Vec<f64> =
            (0..n).map(|x| lanczos((x as f64 + xmin as f64 - center + 0.5) * ss)).collect();
        let total: f64 = w.iter().sum();
        if total != 0.0 {
            for v in w.iter_mut() {
                *v /= total;
            }
        }
        let fixed = w
            .iter()
            .map(|v| {
                let scaled = v * (1u32 << PRECISION_BITS) as f64;
                if *v < 0.0 {
                    (-0.5 + scaled) as i32
                } else {
                    (0.5 + scaled) as i32
                }
            })
            .collect();
        out.push((xmin as usize, fixed));
    }
    out
}

#[inline]
fn clip8(v: i32) -> u8 {
    (v >> PRECISION_BITS).clamp(0, 255) as u8
}

/// `img.resize((out_w, out_h), Image.LANCZOS)`
pub fn resize_lanczos(src: &Img, out_w: u32, out_h: u32) -> Img {
    if out_w == 0 || out_h == 0 || src.w == 0 || src.h == 0 {
        return Img::new(out_w, out_h);
    }
    let (sw, sh) = (src.w as usize, src.h as usize);
    let (ow, oh) = (out_w as usize, out_h as usize);
    // horizontal pass
    let mid = if ow != sw {
        let coefs = coefficients(sw, ow);
        let mut mid = vec![0u8; ow * sh * 3];
        for y in 0..sh {
            let row = &src.rgb[y * sw * 3..(y + 1) * sw * 3];
            let dst = &mut mid[y * ow * 3..(y + 1) * ow * 3];
            for (xx, (xmin, k)) in coefs.iter().enumerate() {
                let mut acc = [1i32 << (PRECISION_BITS - 1); 3];
                for (j, kv) in k.iter().enumerate() {
                    let p = (xmin + j) * 3;
                    acc[0] += row[p] as i32 * kv;
                    acc[1] += row[p + 1] as i32 * kv;
                    acc[2] += row[p + 2] as i32 * kv;
                }
                dst[xx * 3] = clip8(acc[0]);
                dst[xx * 3 + 1] = clip8(acc[1]);
                dst[xx * 3 + 2] = clip8(acc[2]);
            }
        }
        mid
    } else {
        src.rgb.clone()
    };
    // vertical pass
    if oh == sh {
        return Img { w: out_w, h: out_h, rgb: mid };
    }
    let coefs = coefficients(sh, oh);
    let mut out = vec![0u8; ow * oh * 3];
    for (yy, (ymin, k)) in coefs.iter().enumerate() {
        let dst = &mut out[yy * ow * 3..(yy + 1) * ow * 3];
        let mut acc = vec![1i32 << (PRECISION_BITS - 1); ow * 3];
        for (j, kv) in k.iter().enumerate() {
            let row = &mid[(ymin + j) * ow * 3..(ymin + j + 1) * ow * 3];
            for (a, v) in acc.iter_mut().zip(row.iter()) {
                *a += *v as i32 * kv;
            }
        }
        for (d, a) in dst.iter_mut().zip(acc.iter()) {
            *d = clip8(*a);
        }
    }
    Img { w: out_w, h: out_h, rgb: out }
}

// ---------------------------------------------------------------- text (Arial 11 px, like ImageFont.truetype("arial.ttf", 11))

/// Draws short labels with GDI into a small bitmap and blends them onto an image.
#[cfg(windows)]
pub struct Labels {
    dc: ffi::HDC,
    font: ffi::HGDIOBJ,
    old_font: ffi::HGDIOBJ,
}

#[cfg(windows)]
impl Labels {
    pub fn new() -> Labels {
        Labels::with_height(11)
    }

    /// Arial with an em of `px` pixels.
    pub fn with_height(px: i32) -> Labels {
        unsafe {
            let dc = ffi::CreateCompatibleDC(0);
            let face = ffi::wide("Arial");
            // a negative height is the em size; antialiased quality (4)
            let font = ffi::CreateFontW(-px, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 4, 0, face.as_ptr());
            let old_font = if dc != 0 && font != 0 { ffi::SelectObject(dc, font) } else { 0 };
            Labels { dc, font, old_font }
        }
    }

    /// `d.text((x, y), text, fill=c, font=font)`
    pub fn draw(&self, img: &mut Img, x: i64, y: i64, text: &str, c: Rgba) {
        if self.dc == 0 || self.font == 0 || text.is_empty() {
            return;
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let mut size = ffi::SIZE::default();
            if ffi::GetTextExtentPoint32W(self.dc, wide.as_ptr(), wide.len() as i32, &mut size) == 0 {
                return;
            }
            let (w, h) = (size.cx.clamp(1, 2000), size.cy.clamp(1, 200));
            let bmi = ffi::BITMAPINFOHEADER {
                biSize: std::mem::size_of::<ffi::BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = ffi::CreateDIBSection(self.dc, &bmi, 0, &mut bits, 0, 0);
            if dib == 0 || bits.is_null() {
                return;
            }
            let old = ffi::SelectObject(self.dc, dib);
            ffi::SetBkMode(self.dc, 1);
            ffi::SetTextColor(self.dc, 0x00FF_FFFF);
            ffi::TextOutW(self.dc, 0, 0, wide.as_ptr(), wide.len() as i32);
            ffi::GdiFlush();
            let px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
            for row in 0..h as i64 {
                for col in 0..w as i64 {
                    let coverage = px[((row * w as i64 + col) * 4 + 1) as usize] as u32;
                    if coverage > 0 {
                        let alpha = (coverage * c.3 as u32 / 255) as u8;
                        img.blend(x + col, y + row, (c.0, c.1, c.2, alpha));
                    }
                }
            }
            ffi::SelectObject(self.dc, old);
            ffi::DeleteObject(dib);
        }
    }
}

#[cfg(windows)]
impl Default for Labels {
    fn default() -> Self {
        Labels::new()
    }
}

#[cfg(windows)]
impl Drop for Labels {
    fn drop(&mut self) {
        unsafe {
            if self.dc != 0 {
                if self.old_font != 0 {
                    ffi::SelectObject(self.dc, self.old_font);
                }
                ffi::DeleteDC(self.dc);
            }
            if self.font != 0 {
                ffi::DeleteObject(self.font);
            }
        }
    }
}

// ---------------------------------------------------------------- output

/// `img.save(buf, "JPEG", quality=quality)` (4:2:0 chroma, standard tables)
pub fn jpeg(img: &Img, quality: u8) -> Res<Vec<u8>> {
    if img.w == 0 || img.h == 0 || img.w > 65535 || img.h > 65535 {
        return Err(Error::value(format!("cannot write a {}x{} JPEG", img.w, img.h)));
    }
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, quality);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
    enc.encode(&img.rgb, img.w as u16, img.h as u16, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| Error::exc("OSError", format!("encoder error: {e}")))?;
    Ok(out)
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn jpeg_base64(img: &Img, quality: u8) -> Res<String> {
    Ok(base64(&jpeg(img, quality)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn resize_keeps_flat_colors_and_sizes() {
        let mut img = Img::new(400, 200);
        for px in img.rgb.chunks_exact_mut(3) {
            px.copy_from_slice(&[10, 200, 90]);
        }
        let small = resize_lanczos(&img, 100, 50);
        assert_eq!((small.w, small.h), (100, 50));
        assert!(small.rgb.chunks_exact(3).all(|px| px == [10, 200, 90]));
        let big = resize_lanczos(&small, 300, 150);
        assert_eq!((big.w, big.h), (300, 150));
        assert!(big.rgb.chunks_exact(3).all(|px| px == [10, 200, 90]));
    }

    #[test]
    fn drawing_blends_and_clips() {
        let mut img = Img::new(20, 20);
        img.fill_rect(-5, -5, 3, 3, (255, 0, 0, 255));
        assert_eq!(&img.rgb[..3], &[255, 0, 0]);
        img.blend(10, 10, (255, 255, 255, 60));
        let i = (10 * 20 + 10) * 3;
        assert_eq!(img.rgb[i], 60);
        img.outline_rect(5, 5, 8, 8, (0, 255, 0, 255));
        assert_eq!(&img.rgb[(5 * 20 + 5) * 3..(5 * 20 + 5) * 3 + 3], &[0, 255, 0]);
        assert_eq!(&img.rgb[(6 * 20 + 6) * 3..(6 * 20 + 6) * 3 + 3], &[0, 0, 0]);
        assert!(!Img::new(8, 8).is_blank());
        assert!(Img::new(32, 32).is_blank());
        let data = jpeg(&img, 82).unwrap();
        assert_eq!(&data[..2], &[0xff, 0xd8]);
    }
}
