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

/// `DWMWA_TRANSITIONS_FORCEDISABLED` (dwmapi.h).
const DWMWA_TRANSITIONS_FORCEDISABLED: u32 = 3;

#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: *mut std::ffi::c_void,
        dwAttribute: u32,
        pvAttribute: *const std::ffi::c_void,
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
}

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
