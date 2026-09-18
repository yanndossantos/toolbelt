//! Le `BitBlt` proprement dit. Windows uniquement.

use std::ffi::c_void;
use std::ptr::null_mut;

use crate::sys::*;
use crate::{CaptureError, Result, Screenshot};

/// Libère le DC écran. Les gardes existent parce que la fonction a six
/// sorties en erreur : un `ReleaseDC` oublié sur l'une d'elles fuit un
/// handle GDI à chaque capture ratée, et le quota par processus est fini.
struct ScreenDc(HDC);
impl Drop for ScreenDc {
    fn drop(&mut self) {
        unsafe { ReleaseDC(null_mut(), self.0) };
    }
}

struct MemDc(HDC);
impl Drop for MemDc {
    fn drop(&mut self) {
        unsafe { DeleteDC(self.0) };
    }
}

struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.0) };
    }
}

/// Capture le bureau virtuel entier.
///
/// Les dimensions sont en pixels physiques : le processus est déclaré
/// per-monitor DPI aware par le manifeste de Tauri. Sans cette déclaration
/// Windows mentirait sur les métriques et rendrait une image étirée par le
/// système, donc floue.
pub fn capture_virtual_screen() -> Result<Screenshot> {
    unsafe {
        let origin_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let origin_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        if width <= 0 || height <= 0 {
            return Err(CaptureError::NoScreen);
        }

        let screen = GetDC(null_mut());
        if screen.is_null() {
            return Err(CaptureError::Win32("GetDC", GetLastError()));
        }
        let screen = ScreenDc(screen);

        let mem = CreateCompatibleDC(screen.0);
        if mem.is_null() {
            return Err(CaptureError::Win32("CreateCompatibleDC", GetLastError()));
        }
        let mem = MemDc(mem);

        // Compatible avec le DC **écran**, pas avec le DC mémoire : un DC
        // mémoire neuf porte une bitmap monochrome 1×1, et la profondeur
        // héritée serait 1 bit.
        let bitmap = CreateCompatibleBitmap(screen.0, width, height);
        if bitmap.is_null() {
            return Err(CaptureError::Win32(
                "CreateCompatibleBitmap",
                GetLastError(),
            ));
        }
        let bitmap = Bitmap(bitmap);

        let previous = SelectObject(mem.0, bitmap.0);

        let blitted = BitBlt(
            mem.0,
            0,
            0,
            width,
            height,
            screen.0,
            origin_x,
            origin_y,
            SRCCOPY | CAPTUREBLT,
        );

        let result = if blitted == 0 {
            Err(CaptureError::Win32("BitBlt", GetLastError()))
        } else {
            read_pixels(mem.0, bitmap.0, width, height)
        };

        // Une bitmap encore sélectionnée dans un DC ne se détruit pas. Il
        // faut la désélectionner avant que les gardes ne s'exécutent.
        SelectObject(mem.0, previous);

        let rgb = result?;

        Ok(Screenshot {
            width: width as u32,
            height: height as u32,
            origin_x,
            origin_y,
            rgb,
        })
    }
}

/// Relit la bitmap en 32 bits et la convertit en RVB.
unsafe fn read_pixels(dc: HDC, bitmap: HBITMAP, width: i32, height: i32) -> Result<Vec<u8>> {
    let mut info = BITMAPINFO::default();
    info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as DWORD;
    info.bmiHeader.biWidth = width;
    // Hauteur négative = DIB de haut en bas. Sans ça Windows rend les lignes
    // dans l'ordre inverse, convention héritée des bitmaps OS/2.
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;

    // En 32 bits par pixel la ligne est toujours alignée sur 4 octets :
    // aucun remplissage de fin de ligne à sauter.
    let mut bgrx = vec![0u8; (width as usize) * (height as usize) * 4];

    let lines = GetDIBits(
        dc,
        bitmap,
        0,
        height as UINT,
        bgrx.as_mut_ptr() as *mut c_void,
        &mut info,
        DIB_RGB_COLORS,
    );
    if lines == 0 {
        return Err(CaptureError::Win32("GetDIBits", GetLastError()));
    }

    // GDI range les composantes en BGR ; PNG les veut en RGB. Le quatrième
    // octet est ignoré : voir le commentaire de `Screenshot`.
    let mut rgb = Vec::with_capacity((width as usize) * (height as usize) * 3);
    for px in bgrx.chunks_exact(4) {
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    Ok(rgb)
}
