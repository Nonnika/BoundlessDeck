//! PDF pages rendered by CoreGraphics: a page at a time, at the width the
//! block shows it, in BGRA as GPUI's images are.

use std::ffi::c_void;
use std::path::Path;
use std::sync::Mutex;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

type Ref = *mut c_void;

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(alloc: Ref, buf: *const u8, len: isize, dir: bool) -> Ref;
    fn CFRelease(r: Ref);
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPDFDocumentCreateWithURL(url: Ref) -> Ref;
    fn CGPDFDocumentGetNumberOfPages(doc: Ref) -> usize;
    fn CGPDFDocumentGetPage(doc: Ref, n: usize) -> Ref;
    fn CGPDFDocumentRelease(doc: Ref);
    fn CGPDFPageGetBoxRect(page: Ref, box_: i32) -> CGRect;
    fn CGPDFPageGetRotationAngle(page: Ref) -> i32;
    fn CGColorSpaceCreateDeviceRGB() -> Ref;
    fn CGColorSpaceRelease(space: Ref);
    fn CGBitmapContextCreate(data: *mut c_void, w: usize, h: usize, bpc: usize, bpr: usize, space: Ref, info: u32) -> Ref;
    fn CGContextRelease(ctx: Ref);
    fn CGContextSetRGBFillColor(ctx: Ref, r: f64, g: f64, b: f64, a: f64);
    fn CGContextFillRect(ctx: Ref, r: CGRect);
    fn CGContextTranslateCTM(ctx: Ref, x: f64, y: f64);
    fn CGContextScaleCTM(ctx: Ref, x: f64, y: f64);
    fn CGContextRotateCTM(ctx: Ref, a: f64);
    fn CGContextDrawPDFPage(ctx: Ref, page: Ref);
}

/// A PDF open for its preview.
pub struct Doc {
    doc: Mutex<Ref>,
    pages: usize,
}

// CGPDFDocument is immutable once made; the mutex keeps renders one at a time.
unsafe impl Send for Doc {}
unsafe impl Sync for Doc {}

/// The most pixels a page renders at.
const MAX_PIXELS: usize = 1 << 24;

impl Doc {
    pub fn open(path: &Path) -> Result<Doc, String> {
        let bytes = path.as_os_str().as_encoded_bytes();
        unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null_mut(), bytes.as_ptr(), bytes.len() as isize, false);
            if url.is_null() {
                return Err("不是可以打开的 PDF".into());
            }
            let doc = CGPDFDocumentCreateWithURL(url);
            CFRelease(url);
            if doc.is_null() {
                return Err("不是可以打开的 PDF".into());
            }
            let pages = CGPDFDocumentGetNumberOfPages(doc);
            if pages == 0 {
                CGPDFDocumentRelease(doc);
                return Err("PDF 没有页面".into());
            }
            Ok(Doc { doc: Mutex::new(doc), pages })
        }
    }

    pub fn pages(&self) -> usize {
        self.pages
    }

    /// Renders page i (from 0) `width` pixels wide: BGRA, premultiplied, on white.
    pub fn render(&self, i: usize, width: usize) -> Result<(usize, usize, Vec<u8>), String> {
        let doc = self.doc.lock().unwrap();
        if doc.is_null() {
            return Err("PDF 已关闭".into());
        }
        if i >= self.pages {
            return Err("没有这一页".into());
        }
        unsafe {
            let page = CGPDFDocumentGetPage(*doc, i + 1);
            if page.is_null() {
                return Err("这一页打不开".into());
            }
            let r = CGPDFPageGetBoxRect(page, 1); // the crop box
            let turn = (CGPDFPageGetRotationAngle(page) % 360 + 360) % 360;
            let (mut pw, mut ph) = (r.w, r.h);
            if turn == 90 || turn == 270 {
                std::mem::swap(&mut pw, &mut ph);
            }
            if pw <= 0. || ph <= 0. {
                return Err("这一页是空的".into());
            }
            let mut w = width.max(16);
            let mut h = (w as f64 * ph / pw).ceil() as usize;
            if w * h > MAX_PIXELS {
                let k = (MAX_PIXELS as f64 / (w * h) as f64).sqrt();
                w = (w as f64 * k) as usize;
                h = (h as f64 * k) as usize;
            }
            let s = w as f64 / pw;
            let mut buf = vec![0u8; w * h * 4];
            let space = CGColorSpaceCreateDeviceRGB();
            // Premultiplied-first, 32-bit little endian: B G R A in memory.
            let info = 2 | (2 << 12);
            let ctx = CGBitmapContextCreate(buf.as_mut_ptr() as *mut c_void, w, h, 8, w * 4, space, info);
            CGColorSpaceRelease(space);
            if ctx.is_null() {
                return Err("无法绘制这一页".into());
            }
            CGContextSetRGBFillColor(ctx, 1., 1., 1., 1.);
            CGContextFillRect(ctx, CGRect { x: 0., y: 0., w: w as f64, h: h as f64 });
            CGContextScaleCTM(ctx, s, s);
            match turn {
                90 => {
                    CGContextTranslateCTM(ctx, 0., ph);
                    CGContextRotateCTM(ctx, -std::f64::consts::FRAC_PI_2);
                }
                180 => {
                    CGContextTranslateCTM(ctx, pw, ph);
                    CGContextRotateCTM(ctx, std::f64::consts::PI);
                }
                270 => {
                    CGContextTranslateCTM(ctx, pw, 0.);
                    CGContextRotateCTM(ctx, std::f64::consts::FRAC_PI_2);
                }
                _ => {}
            }
            CGContextTranslateCTM(ctx, -r.x, -r.y);
            CGContextDrawPDFPage(ctx, page);
            CGContextRelease(ctx);
            Ok((w, h, buf))
        }
    }
}

impl Drop for Doc {
    fn drop(&mut self) {
        let doc = self.doc.get_mut().unwrap();
        if !doc.is_null() {
            unsafe { CGPDFDocumentRelease(*doc) };
            *doc = std::ptr::null_mut();
        }
    }
}

/// How many pixels wide a page renders for a body w points wide, at 2× and
/// in steps of 256, so that zooming renders it again only now and then.
pub fn width_for(w: f32) -> usize {
    const STEP: usize = 256;
    let px = (w * 2.) as usize;
    px.div_ceil(STEP).max(1).saturating_mul(STEP).clamp(STEP, 4096)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A one-page PDF, 200×100 points, with a black square.
    pub fn write_pdf(path: &Path) {
        let content = "0 0 0 rg 10 10 50 50 re f";
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut out = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.push_str(&format!("{} 0 obj\n{o}\nendobj\n", i + 1));
        }
        let xref = out.len();
        out.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1));
        for off in offsets {
            out.push_str(&format!("{off:010} 00000 n \n"));
        }
        out.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1));
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn renders_a_page() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        write_pdf(&p);
        let doc = Doc::open(&p).unwrap();
        assert_eq!(doc.pages(), 1);
        let (w, h, px) = doc.render(0, 400).unwrap();
        assert_eq!((w, h), (400, 200));
        // Top-left is white; the square sits at the bottom left.
        assert_eq!(&px[0..4], &[255, 255, 255, 255]);
        let at = |x: usize, y: usize| &px[(y * w + x) * 4..(y * w + x) * 4 + 3];
        assert_eq!(at(60, 150), &[0, 0, 0]);
        assert!(doc.render(1, 400).is_err());
    }

    #[test]
    fn not_a_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        std::fs::write(&p, "nope").unwrap();
        assert!(Doc::open(&p).is_err());
    }

    #[test]
    fn widths_go_in_steps() {
        assert_eq!(width_for(10.), 256);
        assert_eq!(width_for(200.), 512);
        assert_eq!(width_for(5000.), 4096);
    }
}
