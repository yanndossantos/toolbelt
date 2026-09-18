//! Raccourci clavier global.
//!
//! `RegisterHotKey` sans fenêtre : le hotkey est alors rattaché au **thread**
//! appelant, et Windows y poste `WM_HOTKEY`. D'où le thread dédié avec sa
//! propre boucle de messages — `GetMessageW` bloque, il ne peut pas partager
//! le thread de l'UI.
//!
//! Pas de `tauri-plugin-global-shortcut` : comme pour `autostart.rs`, l'appel
//! direct tient en trente lignes et fait remonter le code d'erreur Win32
//! jusqu'à l'interface, ce qu'un plugin masquerait.

#![allow(non_snake_case, non_camel_case_types)]

use std::ffi::c_void;
use std::sync::mpsc;

type BOOL = i32;
type DWORD = u32;
type UINT = u32;
type HWND = *mut c_void;

const MOD_SHIFT: UINT = 0x0004;
const MOD_WIN: UINT = 0x0008;
/// Sans ce drapeau, garder le raccourci enfoncé déclencherait une rafale de
/// captures.
const MOD_NOREPEAT: UINT = 0x4000;

const VK_X: UINT = 0x58;
const WM_HOTKEY: UINT = 0x0312;

/// Identifiant du raccourci, libre mais unique dans le thread.
const HOTKEY_ID: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct POINT {
    x: i32,
    y: i32,
}

/// Les SDK récents ajoutent `lPrivate` après `pt`. Le champ est déclaré même
/// s'il n'est jamais lu : `GetMessageW` écrit dans toute la structure, et la
/// déclarer trop courte laisserait Windows écraser la pile.
#[repr(C)]
#[derive(Clone, Copy)]
struct MSG {
    hwnd: HWND,
    message: UINT,
    wParam: usize,
    lParam: isize,
    time: DWORD,
    pt: POINT,
    lPrivate: DWORD,
}

// Default manuel : un pointeur brut n'implémente pas Default.
impl Default for MSG {
    fn default() -> Self {
        Self {
            hwnd: std::ptr::null_mut(),
            message: 0,
            wParam: 0,
            lParam: 0,
            time: 0,
            pt: POINT::default(),
            lPrivate: 0,
        }
    }
}

#[link(name = "user32")]
extern "system" {
    fn RegisterHotKey(hWnd: HWND, id: i32, fsModifiers: UINT, vk: UINT) -> BOOL;
    fn UnregisterHotKey(hWnd: HWND, id: i32) -> BOOL;
    fn GetMessageW(lpMsg: *mut MSG, hWnd: HWND, wMsgFilterMin: UINT, wMsgFilterMax: UINT) -> BOOL;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> DWORD;
}

/// Libellé du raccourci, tel qu'affiché à l'utilisateur.
pub const LABEL: &str = "Win+Shift+X";

/// Enregistre Win+Shift+X et appelle `on_press` à chaque appui.
///
/// Le thread ne s'arrête jamais : il vit aussi longtemps que le processus.
/// Rend l'erreur d'enregistrement, qui vaut d'être affichée — le cas courant
/// est le code 1409, `ERROR_HOTKEY_ALREADY_REGISTERED`, quand une autre
/// application a pris la combinaison avant nous.
pub fn spawn<F>(on_press: F) -> Result<(), String>
where
    F: Fn() + Send + 'static,
{
    let (tx, rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("hotkey".into())
        .spawn(move || unsafe {
            let ok = RegisterHotKey(
                std::ptr::null_mut(),
                HOTKEY_ID,
                MOD_WIN | MOD_SHIFT | MOD_NOREPEAT,
                VK_X,
            );

            if ok == 0 {
                let code = GetLastError();
                let _ = tx.send(Err(match code {
                    1409 => format!("{LABEL} est déjà pris par une autre application"),
                    other => format!("RegisterHotKey a échoué (code {other})"),
                }));
                return;
            }
            let _ = tx.send(Ok(()));

            // `hWnd` nul et filtre nul : on reçoit tous les messages du
            // thread, dont WM_HOTKEY. -1 signale une erreur, 0 un WM_QUIT.
            let mut msg = MSG::default();
            loop {
                let got = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                if got <= 0 {
                    break;
                }
                if msg.message == WM_HOTKEY && msg.wParam == HOTKEY_ID as usize {
                    on_press();
                }
            }

            UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
        })
        .map_err(|e| format!("thread du raccourci : {e}"))?;

    rx.recv()
        .unwrap_or_else(|_| Err("le thread du raccourci s'est arrêté".into()))
}
