//! Déclarations FFI brutes pour la capture d'écran.
//!
//! Écrites à la main pour la même raison que dans le crate `ddc` : ce sont
//! des fonctions plates, et ça évite de deviner lesquels de leurs paramètres
//! windows-rs enveloppe dans un `Option`.

#![allow(non_snake_case, non_camel_case_types)]

use std::ffi::c_void;

pub type BOOL = i32;
pub type DWORD = u32;
pub type WORD = u16;
pub type LONG = i32;
pub type UINT = u32;
pub type HDC = *mut c_void;
pub type HWND = *mut c_void;
pub type HBITMAP = *mut c_void;
pub type HGDIOBJ = *mut c_void;

/// Index de `GetSystemMetrics` décrivant le bureau virtuel, c'est-à-dire le
/// rectangle englobant tous les écrans. L'origine peut être négative : un
/// écran placé à gauche du principal a des x négatifs.
pub const SM_XVIRTUALSCREEN: i32 = 76;
pub const SM_YVIRTUALSCREEN: i32 = 77;
pub const SM_CXVIRTUALSCREEN: i32 = 78;
pub const SM_CYVIRTUALSCREEN: i32 = 79;

pub const SRCCOPY: DWORD = 0x00CC_0020;
/// Sans ce drapeau, les fenêtres en couches (menus, curseurs personnalisés,
/// certains overlays) sont absentes de la capture.
pub const CAPTUREBLT: DWORD = 0x4000_0000;

pub const DIB_RGB_COLORS: UINT = 0;
pub const BI_RGB: DWORD = 0;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BITMAPINFOHEADER {
    pub biSize: DWORD,
    pub biWidth: LONG,
    pub biHeight: LONG,
    pub biPlanes: WORD,
    pub biBitCount: WORD,
    pub biCompression: DWORD,
    pub biSizeImage: DWORD,
    pub biXPelsPerMeter: LONG,
    pub biYPelsPerMeter: LONG,
    pub biClrUsed: DWORD,
    pub biClrImportant: DWORD,
}

/// `BITMAPINFO` complet. Le champ `bmiColors` n'a pas d'usage en 32 bits
/// BI_RGB, mais GetDIBits lit la structure entière : le retirer déborderait.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BITMAPINFO {
    pub bmiHeader: BITMAPINFOHEADER,
    pub bmiColors: [DWORD; 3],
}

#[link(name = "user32")]
extern "system" {
    pub fn GetSystemMetrics(nIndex: i32) -> i32;
    pub fn GetDC(hWnd: HWND) -> HDC;
    pub fn ReleaseDC(hWnd: HWND, hDC: HDC) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    pub fn CreateCompatibleDC(hdc: HDC) -> HDC;
    pub fn CreateCompatibleBitmap(hdc: HDC, cx: i32, cy: i32) -> HBITMAP;
    pub fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
    pub fn DeleteObject(ho: HGDIOBJ) -> BOOL;
    pub fn DeleteDC(hdc: HDC) -> BOOL;
    pub fn BitBlt(
        hdc: HDC,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        hdcSrc: HDC,
        x1: i32,
        y1: i32,
        rop: DWORD,
    ) -> BOOL;
    pub fn GetDIBits(
        hdc: HDC,
        hbm: HBITMAP,
        start: UINT,
        cLines: UINT,
        lpvBits: *mut c_void,
        lpbmi: *mut BITMAPINFO,
        usage: UINT,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetLastError() -> DWORD;
}
