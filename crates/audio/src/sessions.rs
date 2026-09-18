//! Mélangeur : volume par application sur la sortie courante.
//!
//! Chaîne d'API : la sortie par défaut (`IMMDevice`) donne un
//! `IAudioSessionManager2`, qui énumère des `IAudioSessionControl`. Chacun se
//! caste en `IAudioSessionControl2` pour le PID, et en `ISimpleAudioVolume`
//! pour le volume. Tout est public et documenté, contrairement à
//! `IPolicyConfig`.
//!
//! Deux subtilités :
//!
//! - Une application peut ouvrir **plusieurs sessions** (Chrome en a une par
//!   onglet qui produit du son). On les regroupe par PID, sinon la liste
//!   devient illisible.
//! - Les sessions sont attachées à un périphérique. Changer de sortie change
//!   la liste : il faut réénumérer après un basculement.

use std::collections::HashMap;

use serde::Serialize;
use windows::core::{Interface, Result, GUID};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2,
    IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Une application qui joue du son sur la sortie courante.
#[derive(Debug, Clone, Serialize)]
pub struct AudioSession {
    pub pid: u32,
    /// Nom de l'exécutable sans extension, ou « Sons système ».
    pub name: String,
    /// 0.0 à 1.0.
    pub volume: f32,
    pub muted: bool,
    pub is_system: bool,
}

/// Volume général de la sortie, celui du curseur de la barre des tâches.
#[derive(Debug, Clone, Serialize)]
pub struct MasterState {
    pub volume: f32,
    pub muted: bool,
}

/// Contexte d'événement des changements de volume.
///
/// Windows le transmet aux notifications de volume pour qu'un client puisse
/// reconnaître ses propres modifications et ne pas y réagir. Tant qu'on
/// n'écoute pas ces notifications, un pointeur nul convient. À remplacer par
/// un GUID à nous le jour où on implémentera `IAudioSessionNotification`.
const NO_EVENT_CONTEXT: *const GUID = std::ptr::null();

unsafe fn default_render_device() -> Result<IMMDevice> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)
}

// --- Volume général -----------------------------------------------------

pub fn master_state() -> Result<MasterState> {
    unsafe {
        let endpoint: IAudioEndpointVolume = default_render_device()?.Activate(CLSCTX_ALL, None)?;
        Ok(MasterState {
            volume: endpoint.GetMasterVolumeLevelScalar()?,
            muted: endpoint.GetMute()?.as_bool(),
        })
    }
}

pub fn set_master_volume(volume: f32) -> Result<()> {
    unsafe {
        let endpoint: IAudioEndpointVolume = default_render_device()?.Activate(CLSCTX_ALL, None)?;
        endpoint.SetMasterVolumeLevelScalar(volume.clamp(0.0, 1.0), NO_EVENT_CONTEXT)
    }
}

pub fn set_master_mute(muted: bool) -> Result<()> {
    unsafe {
        let endpoint: IAudioEndpointVolume = default_render_device()?.Activate(CLSCTX_ALL, None)?;
        endpoint.SetMute(muted, NO_EVENT_CONTEXT)
    }
}

// --- Sessions -----------------------------------------------------------

/// Liste les applications actives, regroupées par processus.
pub fn list_sessions() -> Result<Vec<AudioSession>> {
    unsafe {
        let manager: IAudioSessionManager2 = default_render_device()?.Activate(CLSCTX_ALL, None)?;
        let enumerator = manager.GetSessionEnumerator()?;
        let count = enumerator.GetCount()?;

        let mut by_pid: HashMap<u32, AudioSession> = HashMap::new();
        // HashMap ne garde pas l'ordre ; on le suit à part pour que la liste
        // ne se réorganise pas sous le curseur à chaque rafraîchissement.
        let mut order: Vec<u32> = Vec::new();

        for i in 0..count {
            let control = match enumerator.GetSession(i) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Une session expirée correspond à une appli déjà fermée.
            if let Ok(state) = control.GetState() {
                if state == AudioSessionStateExpired {
                    continue;
                }
            }

            let control2: IAudioSessionControl2 = match control.cast() {
                Ok(c) => c,
                Err(_) => continue,
            };
            let volume_ctl: ISimpleAudioVolume = match control.cast() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Le PID 0 est la session des sons système.
            let pid = control2.GetProcessId().unwrap_or(0);
            let volume = volume_ctl.GetMasterVolume().unwrap_or(0.0);
            let muted = volume_ctl.GetMute().map(|b| b.as_bool()).unwrap_or(false);

            match by_pid.get_mut(&pid) {
                Some(existing) => {
                    // Plusieurs sessions pour un même processus : on montre la
                    // plus forte, et on ne dit « coupé » que si tout l'est.
                    if volume > existing.volume {
                        existing.volume = volume;
                    }
                    existing.muted = existing.muted && muted;
                }
                None => {
                    order.push(pid);
                    by_pid.insert(
                        pid,
                        AudioSession {
                            pid,
                            name: if pid == 0 {
                                String::from("Sons système")
                            } else {
                                process_name(pid)
                            },
                            volume,
                            muted,
                            is_system: pid == 0,
                        },
                    );
                }
            }
        }

        Ok(order
            .into_iter()
            .filter_map(|pid| by_pid.remove(&pid))
            .collect())
    }
}

pub fn set_session_volume(pid: u32, volume: f32) -> Result<()> {
    let v = volume.clamp(0.0, 1.0);
    for_each_session(pid, |ctl| unsafe {
        let _ = ctl.SetMasterVolume(v, NO_EVENT_CONTEXT);
    })
}

pub fn set_session_mute(pid: u32, muted: bool) -> Result<()> {
    for_each_session(pid, |ctl| unsafe {
        let _ = ctl.SetMute(muted, NO_EVENT_CONTEXT);
    })
}

/// Applique une action à toutes les sessions d'un processus.
fn for_each_session<F>(pid: u32, mut action: F) -> Result<()>
where
    F: FnMut(&ISimpleAudioVolume),
{
    unsafe {
        let manager: IAudioSessionManager2 = default_render_device()?.Activate(CLSCTX_ALL, None)?;
        let enumerator = manager.GetSessionEnumerator()?;

        for i in 0..enumerator.GetCount()? {
            let control = match enumerator.GetSession(i) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let control2: IAudioSessionControl2 = match control.cast() {
                Ok(c) => c,
                Err(_) => continue,
            };
            if control2.GetProcessId().unwrap_or(0) != pid {
                continue;
            }
            if let Ok(volume_ctl) = control.cast::<ISimpleAudioVolume>() {
                action(&volume_ctl);
            }
        }
        Ok(())
    }
}

// --- Nom de processus ---------------------------------------------------

/// Chemin complet de l'exécutable d'un processus.
///
/// Public parce que le module `icons` en a besoin pour aller y chercher
/// l'icône.
pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        // PROCESS_QUERY_LIMITED_INFORMATION suffit et passe sans élévation,
        // contrairement à PROCESS_QUERY_INFORMATION.
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buf = vec![0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);

        if !ok || len == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// `IAudioSessionControl::GetDisplayName` est vide pour presque toutes les
/// applications. Le nom fiable vient du chemin de l'exécutable.
fn process_name(pid: u32) -> String {
    let fallback = format!("PID {pid}");

    let path = match process_path(pid) {
        Some(p) => p,
        None => return fallback,
    };

    let file = path
        .rsplit(|c| c == '\\' || c == '/')
        .next()
        .unwrap_or("")
        .to_string();

    let stem = file
        .strip_suffix(".exe")
        .or_else(|| file.strip_suffix(".EXE"))
        .unwrap_or(&file);

    if stem.is_empty() {
        fallback
    } else {
        capitalize(stem)
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
