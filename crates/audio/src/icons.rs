//! Extraction de l'icône d'une application.
//!
//! Chaîne : PID → chemin de l'exécutable → `ExtractIconExW` → `HICON` →
//! bitmap RGBA via `GetDIBits`.
//!
//! Comme pour le crate `ddc`, les appels Win32 sont déclarés à la main. Ce
//! sont des fonctions plates, et ça évite de deviner lesquels de leurs
//! paramètres windows-rs enveloppe dans un `Option`.

#![allow(non_snake_case, non_camel_case_types)]

use std::ffi::c_void;

use serde::Serialize;

use crate::sessions::process_path;

/// Icône d'application prête pour le frontend.
#[derive(Debug, Clone, Serialize)]
pub struct IconRgba {
    pub width: u32,
    pub height: u32,
    /// Pixels RGBA 8 bits, en base64. Le frontend les repose dans un canvas.
    pub rgba: String,
}

// --- FFI ----------------------------------------------------------------

type BOOL = i32;
type DWORD = u32;
type WORD = u16;
type LONG = i32;
type HANDLE = *mut c_void;
type HICON = *mut c_void;
type HBITMAP = *mut c_void;
type HDC = *mut c_void;
type HWND = *mut c_void;

// Default manuel : un pointeur brut n'implémente pas Default.
#[repr(C)]
#[derive(Clone, Copy)]
struct ICONINFO {
    fIcon: BOOL,
    xHotspot: DWORD,
    yHotspot: DWORD,
    hbmMask: HBITMAP,
    hbmColor: HBITMAP,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BITMAP {
    bmType: LONG,
    bmWidth: LONG,
    bmHeight: LONG,
    bmWidthBytes: LONG,
    bmPlanes: WORD,
    bmBitsPixel: WORD,
    bmBits: *mut c_void,
}

impl Default for ICONINFO {
    fn default() -> Self {
        Self {
            fIcon: 0,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: std::ptr::null_mut(),
            hbmColor: std::ptr::null_mut(),
        }
    }
}

impl Default for BITMAP {
    fn default() -> Self {
        Self {
            bmType: 0,
            bmWidth: 0,
            bmHeight: 0,
            bmWidthBytes: 0,
            bmPlanes: 0,
            bmBitsPixel: 0,
            bmBits: std::ptr::null_mut(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BITMAPINFOHEADER {
    biSize: DWORD,
    biWidth: LONG,
    biHeight: LONG,
    biPlanes: WORD,
    biBitCount: WORD,
    biCompression: DWORD,
    biSizeImage: DWORD,
    biXPelsPerMeter: LONG,
    biYPelsPerMeter: LONG,
    biClrUsed: DWORD,
    biClrImportant: DWORD,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RGBQUAD {
    rgbBlue: u8,
    rgbGreen: u8,
    rgbRed: u8,
    rgbReserved: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BITMAPINFO {
    bmiHeader: BITMAPINFOHEADER,
    bmiColors: [RGBQUAD; 1],
}

const BI_RGB: DWORD = 0;
const DIB_RGB_COLORS: u32 = 0;

#[link(name = "shell32")]
extern "system" {
    fn ExtractIconExW(
        lpszFile: *const u16,
        nIconIndex: i32,
        phiconLarge: *mut HICON,
        phiconSmall: *mut HICON,
        nIcons: u32,
    ) -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn GetIconInfo(hIcon: HICON, piconinfo: *mut ICONINFO) -> BOOL;
    fn DestroyIcon(hIcon: HICON) -> BOOL;
    fn GetDC(hWnd: HWND) -> HDC;
    fn ReleaseDC(hWnd: HWND, hDC: HDC) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn GetObjectW(h: HANDLE, c: i32, pv: *mut c_void) -> i32;
    fn GetDIBits(
        hdc: HDC,
        hbm: HBITMAP,
        start: u32,
        cLines: u32,
        lpvBits: *mut c_void,
        lpbmi: *mut BITMAPINFO,
        usage: u32,
    ) -> i32;
    fn DeleteObject(ho: HANDLE) -> BOOL;
}

// --- API ----------------------------------------------------------------

/// Icône d'un processus, ou `None` si l'exécutable est inaccessible ou
/// n'embarque pas d'icône.
pub fn process_icon(pid: u32) -> Option<IconRgba> {
    let path = process_path(pid)?;
    unsafe { extract(&path) }
}

unsafe fn extract(path: &str) -> Option<IconRgba> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    let mut large: HICON = std::ptr::null_mut();
    let count = ExtractIconExW(wide.as_ptr(), 0, &mut large, std::ptr::null_mut(), 1);
    if count == 0 || large.is_null() {
        return None;
    }

    let out = to_rgba(large);
    DestroyIcon(large);
    out
}

unsafe fn to_rgba(icon: HICON) -> Option<IconRgba> {
    let mut info = ICONINFO::default();
    if GetIconInfo(icon, &mut info) == 0 || info.hbmColor.is_null() {
        return None;
    }

    let mut bmp = BITMAP::default();
    let got = GetObjectW(
        info.hbmColor,
        std::mem::size_of::<BITMAP>() as i32,
        &mut bmp as *mut BITMAP as *mut c_void,
    );

    let result = if got != 0 && bmp.bmWidth > 0 && bmp.bmHeight > 0 {
        read_pixels(&info, &bmp)
    } else {
        None
    };

    // Les bitmaps renvoyés par GetIconInfo sont des copies : à nous de les
    // libérer, sinon on fuit deux handles GDI par icône.
    if !info.hbmColor.is_null() {
        DeleteObject(info.hbmColor);
    }
    if !info.hbmMask.is_null() {
        DeleteObject(info.hbmMask);
    }

    result
}

unsafe fn read_pixels(info: &ICONINFO, bmp: &BITMAP) -> Option<IconRgba> {
    let w = bmp.bmWidth;
    let h = bmp.bmHeight;

    let mut bi = BITMAPINFO::default();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as DWORD;
    bi.bmiHeader.biWidth = w;
    // Hauteur négative = image dans le sens de lecture (haut vers bas).
    // Sans ça l'icône sort à l'envers.
    bi.bmiHeader.biHeight = -h;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB;

    let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];

    let screen = GetDC(std::ptr::null_mut());
    let lines = GetDIBits(
        screen,
        info.hbmColor,
        0,
        h as u32,
        buf.as_mut_ptr() as *mut c_void,
        &mut bi,
        DIB_RGB_COLORS,
    );
    ReleaseDC(std::ptr::null_mut(), screen);

    if lines == 0 {
        return None;
    }

    // Win32 rend du BGRA, le canvas attend du RGBA.
    let mut opaque = false;
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
        if px[3] != 0 {
            opaque = true;
        }
    }

    // Les vieilles icônes 24 bits n'ont pas de canal alpha : tout est à zéro,
    // ce qui donnerait une image entièrement transparente.
    if !opaque {
        for px in buf.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }

    Some(IconRgba {
        width: w as u32,
        height: h as u32,
        rgba: base64(&buf),
    })
}

// --- base64 -------------------------------------------------------------

/// Encodage base64 standard. Écrit à la main plutôt qu'ajouter une
/// dépendance pour vingt lignes.
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_vecteurs_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_binaire() {
        assert_eq!(base64(&[0x00, 0x00, 0x00]), "AAAA");
        assert_eq!(base64(&[0xff, 0xff, 0xff]), "////");
        assert_eq!(base64(&[0xfb, 0xff, 0xbf]), "+/+/");
    }
}
