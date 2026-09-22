//! Capture d'écran : de l'appui sur le raccourci au fichier PNG.
//!
//! L'ordre des opérations est capture d'abord, sélection ensuite. Les pixels
//! sont figés avant que l'overlay ne s'affiche, ce qui garantit qu'il ne peut
//! pas se photographier lui-même — et rend la sélection instantanée, puisque
//! le recadrage se fait en mémoire.

use std::path::PathBuf;
use std::sync::Mutex;

use toolbelt_capture::Screenshot;

/// Image figée au moment de l'appui, en attente d'un recadrage.
///
/// `None` quand aucune capture n'est en cours. Le buffer pèse la taille du
/// bureau virtuel en RVB — 25 Mo pour 6000×1440 — d'où la remise à `None`
/// dès que la sélection est validée ou annulée.
#[derive(Default)]
pub struct Pending(pub Mutex<Option<Screenshot>>);

/// Erreur d'enregistrement du raccourci, à afficher dans le flyout.
///
/// Même logique que `get_autostart` : un raccourci qui n'a pas pu être pris
/// doit se voir, sinon l'utilisateur appuie dans le vide sans comprendre.
#[derive(Default)]
pub struct HotkeyError(pub Mutex<Option<String>>);

/// Dossier de destination : `%USERPROFILE%\Pictures\Toolbelt Screenshots`.
///
/// Pas de `SHGetKnownFolderPath` : ça tirerait COM pour lire un chemin que
/// la variable d'environnement donne déjà. Le dossier est créé à l'écriture.
pub fn output_dir() -> Option<PathBuf> {
    let profile = std::env::var_os("USERPROFILE")?;
    Some(
        PathBuf::from(profile)
            .join("Pictures")
            .join("Toolbelt Screenshots"),
    )
}

/// Nom de fichier horodaté, trié chronologiquement par ordre alphabétique.
pub fn file_name() -> String {
    let t = local_time();
    format!(
        "Capture {:04}-{:02}-{:02} {:02}{:02}{:02}.png",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// DWM attributes (dwmapi.h).
const DWMWA_TRANSITIONS_FORCEDISABLED: u32 = 3;
const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;

#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: *mut std::ffi::c_void,
        dwAttribute: u32,
        pvAttribute: *const std::ffi::c_void,
        cbAttribute: u32,
    ) -> i32;
    fn DwmGetWindowAttribute(
        hwnd: *mut std::ffi::c_void,
        dwAttribute: u32,
        pvAttribute: *mut std::ffi::c_void,
        cbAttribute: u32,
    ) -> i32;
}

/// Supprime l'animation d'ouverture que Windows applique aux fenêtres.
///
/// Cette animation déploie la fenêtre depuis son centre. Pour l'overlay, qui
/// couvre le bureau virtuel entier, ce centre tombe à la jointure entre deux
/// écrans : chacun voit l'assombrissement arriver de travers. Coupée ici, le
/// fondu est fait en CSS par `overlay.html`, uniformément.
///
/// Rend le `HRESULT` : 0 = S_OK.
pub fn disable_open_animation(hwnd: *mut std::ffi::c_void) -> i32 {
    // BOOL Win32, pas un bool Rust : 4 octets, pas 1.
    let yes: i32 = 1;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &yes as *const i32 as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        )
    }
}

/// `GWL_EXSTYLE` et `WS_EX_NOACTIVATE` (winuser.h).
const GWL_EXSTYLE: i32 = -20;
const WS_EX_NOACTIVATE: isize = 0x0800_0000;

#[link(name = "user32")]
extern "system" {
    fn GetWindowLongPtrW(hwnd: *mut std::ffi::c_void, nIndex: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: *mut std::ffi::c_void, nIndex: i32, dwNewLong: isize) -> isize;
    fn GetCursorPos(point: *mut POINT) -> i32;
    fn GetWindow(hwnd: *mut std::ffi::c_void, command: u32) -> *mut std::ffi::c_void;
    fn GetAncestor(hwnd: *mut std::ffi::c_void, flags: u32) -> *mut std::ffi::c_void;
    fn IsWindowVisible(hwnd: *mut std::ffi::c_void) -> i32;
    fn GetWindowRect(hwnd: *mut std::ffi::c_void, rect: *mut RECT) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

const GA_ROOT: u32 = 2;
const GW_HWNDNEXT: u32 = 2;

/// Interdit à la fenêtre de prendre le focus, définitivement.
///
/// `show()` de Tauri passe par `ShowWindow(SW_SHOW)`, qui **active** la
/// fenêtre. Pour une confirmation qui s'affiche pendant qu'on tape ailleurs,
/// c'est inacceptable : le clavier partirait dans le vide. `WS_EX_NOACTIVATE`
/// neutralise l'activation à la source, quel que soit l'appel qui affiche.
///
/// Le style est posé une fois à la création, pas à chaque affichage : c'est
/// un attribut de la fenêtre, pas un état.
pub fn never_activate(hwnd: *mut std::ffi::c_void) {
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE);
    }
}

unsafe fn visible_window_rect(hwnd: *mut std::ffi::c_void) -> Option<RECT> {
    let mut rect = RECT::default();
    let result = DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS,
        &mut rect as *mut RECT as *mut std::ffi::c_void,
        std::mem::size_of::<RECT>() as u32,
    );
    if result == 0 && rect.right > rect.left && rect.bottom > rect.top {
        Some(rect)
    } else if GetWindowRect(hwnd, &mut rect) != 0
        && rect.right > rect.left
        && rect.bottom > rect.top
    {
        Some(rect)
    } else {
        None
    }
}

/// Renvoie le rectangle de la fenêtre racine sous le curseur, en pixels écran.
/// La fenêtre d'overlay est ignorée : elle recouvre nécessairement le point
/// du curseur pendant la sélection.
pub fn hovered_window(cursor_excluded: *mut std::ffi::c_void) -> Option<(i32, i32, i32, i32)> {
    unsafe {
        let mut point = POINT::default();
        if GetCursorPos(&mut point) == 0 {
            return None;
        }
        // L'overlay est au sommet de l'ordre Z. Parcourir les fenêtres qui le
        // suivent permet de trouver la première fenêtre visible sous le point,
        // alors qu'un hit-test direct ne verrait que l'overlay lui-même.
        let mut hwnd = GetWindow(cursor_excluded, GW_HWNDNEXT);
        while !hwnd.is_null() {
            if IsWindowVisible(hwnd) != 0 {
                if let Some(rect) = visible_window_rect(hwnd) {
                    if point.x >= rect.left
                        && point.x < rect.right
                        && point.y >= rect.top
                        && point.y < rect.bottom
                    {
                        let root = GetAncestor(hwnd, GA_ROOT);
                        if !root.is_null() && root != cursor_excluded {
                            return Some((
                                rect.left,
                                rect.top,
                                rect.right - rect.left,
                                rect.bottom - rect.top,
                            ));
                        }
                    }
                }
            }
            hwnd = GetWindow(hwnd, GW_HWNDNEXT);
        }
        None
    }
}

#[allow(non_snake_case)]
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct SYSTEMTIME {
    wYear: u16,
    wMonth: u16,
    wDayOfWeek: u16,
    wDay: u16,
    wHour: u16,
    wMinute: u16,
    wSecond: u16,
    wMilliseconds: u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLocalTime(lpSystemTime: *mut SYSTEMTIME);
}

/// L'heure locale via Win32 plutôt qu'un crate de dates : on a besoin de six
/// entiers, pas d'une bibliothèque de calendrier.
fn local_time() -> SYSTEMTIME {
    let mut t = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut t) };
    t
}
