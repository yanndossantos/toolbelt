//! Démarrage automatique, par écriture directe dans le registre.
//!
//! Windows lance tout ce qui est listé sous
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` à l'ouverture de
//! session. Pas besoin de droits administrateur, pas de service, pas de
//! tâche planifiée : une valeur chaîne contenant le chemin de l'exécutable.
//!
//! Écrit à la main plutôt qu'avec un plugin, pour deux raisons : le
//! comportement est entièrement visible ici, et les erreurs remontent avec
//! un code que l'on peut afficher au lieu de disparaître.

#![allow(non_snake_case, non_camel_case_types)]

use std::ffi::c_void;

type HKEY = *mut c_void;
type LSTATUS = i32;
type DWORD = u32;

const HKEY_CURRENT_USER: HKEY = 0x8000_0001u32 as usize as HKEY;
const KEY_READ_WRITE: DWORD = 0x2001F;
const REG_SZ: DWORD = 1;
const ERROR_SUCCESS: LSTATUS = 0;
const ERROR_FILE_NOT_FOUND: LSTATUS = 2;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Nom de la valeur. Le changer casserait le lien avec une entrée déjà posée.
const VALUE_NAME: &str = "Toolbelt";

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(
        hKey: HKEY,
        lpSubKey: *const u16,
        ulOptions: DWORD,
        samDesired: DWORD,
        phkResult: *mut HKEY,
    ) -> LSTATUS;

    fn RegSetValueExW(
        hKey: HKEY,
        lpValueName: *const u16,
        Reserved: DWORD,
        dwType: DWORD,
        lpData: *const u8,
        cbData: DWORD,
    ) -> LSTATUS;

    fn RegQueryValueExW(
        hKey: HKEY,
        lpValueName: *const u16,
        lpReserved: *mut DWORD,
        lpType: *mut DWORD,
        lpData: *mut u8,
        lpcbData: *mut DWORD,
    ) -> LSTATUS;

    fn RegDeleteValueW(hKey: HKEY, lpValueName: *const u16) -> LSTATUS;

    fn RegCloseKey(hKey: HKEY) -> LSTATUS;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Garde RAII sur la clé Run.
struct RunKey(HKEY);

impl RunKey {
    fn open() -> Result<Self, String> {
        let sub = wide(RUN_KEY);
        let mut handle: HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                0,
                KEY_READ_WRITE,
                &mut handle,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!("ouverture de la clé Run refusée (code {status})"));
        }
        Ok(RunKey(handle))
    }
}

impl Drop for RunKey {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

/// Chemin de l'exécutable en cours, entre guillemets.
///
/// Les guillemets ne sont pas cosmétiques : sans eux, un chemin contenant un
/// espace serait coupé au premier espace au démarrage.
pub fn current_command() -> Result<String, String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("chemin de l'exécutable introuvable : {e}"))?;
    Ok(format!("\"{}\"", exe.display()))
}

/// Valeur actuellement enregistrée, ou `None` si l'entrée n'existe pas.
pub fn registered_command() -> Result<Option<String>, String> {
    let key = RunKey::open()?;
    let name = wide(VALUE_NAME);

    let mut size: DWORD = 0;
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(format!("lecture de la valeur refusée (code {status})"));
    }

    let mut buf = vec![0u8; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            buf.as_mut_ptr(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!("lecture de la valeur refusée (code {status})"));
    }

    // Le registre rend des octets ; la valeur est de l'UTF-16.
    let units: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());

    Ok(Some(String::from_utf16_lossy(&units[..end])))
}

pub fn set(enabled: bool) -> Result<(), String> {
    let key = RunKey::open()?;
    let name = wide(VALUE_NAME);

    if !enabled {
        let status = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        return Err(format!("suppression de la valeur refusée (code {status})"));
    }

    let command = wide(&current_command()?);
    // cbData compte des octets, NUL final inclus.
    let bytes = command.len() * 2;

    let status = unsafe {
        RegSetValueExW(
            key.0,
            name.as_ptr(),
            0,
            REG_SZ,
            command.as_ptr() as *const u8,
            bytes as DWORD,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!("écriture de la valeur refusée (code {status})"));
    }
    Ok(())
}
