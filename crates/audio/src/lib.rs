//! Énumération des sorties audio et changement du périphérique par défaut.
//!
//! Lecture : API publique Core Audio (`IMMDeviceEnumerator`).
//! Écriture : `IPolicyConfig`, non documentée — voir `policy_config.rs`.

pub mod icons;
pub mod policy_config;
pub mod sessions;

use std::ffi::c_void;

use windows::core::{Result, BSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Media::Audio::{
    eCapture, eMultimedia, eRender, EDataFlow, IMMDeviceEnumerator, MMDeviceEnumerator,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};

/// Un point de terminaison audio (une sortie ou une entrée).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AudioDevice {
    /// ID opaque et stable, ex. `{0.0.0.00000000}.{guid}`.
    /// C'est lui qu'il faut persister, jamais le nom.
    pub id: String,
    /// Nom lisible, ex. « Haut-parleurs (Realtek) ».
    pub name: String,
    pub is_default: bool,
}

/// Sens du flux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Sorties (haut-parleurs, casque, HDMI).
    Render,
    /// Entrées (micros).
    Capture,
}

impl Direction {
    fn as_dataflow(self) -> EDataFlow {
        match self {
            Direction::Render => eRender,
            Direction::Capture => eCapture,
        }
    }
}

/// Garde RAII sur l'initialisation COM du thread courant.
///
/// COM est par thread : chaque thread qui touche à ces fonctions doit avoir
/// appelé ça d'abord. MTA convient ici ; une UI aurait besoin de STA.
pub struct ComGuard {
    /// Faux quand le thread était déjà initialisé dans un autre mode : dans
    /// ce cas ce n'est pas nous qui avons ouvert COM, donc pas à nous de le
    /// fermer.
    owns: bool,
}

impl ComGuard {
    pub fn new() -> Result<Self> {
        unsafe {
            let hr = CoInitializeEx(None, COINIT_MULTITHREADED);

            // Un thread déjà initialisé en STA (ce que fait Tauri sur
            // certains threads) renvoie RPC_E_CHANGED_MODE. COM reste
            // utilisable, on ne doit simplement pas y toucher.
            if hr == RPC_E_CHANGED_MODE {
                return Ok(ComGuard { owns: false });
            }

            // S_FALSE = déjà initialisé dans le même mode : pas une erreur,
            // mais le compteur a été incrémenté, donc on le décrémentera.
            hr.ok()?;
            Ok(ComGuard { owns: true })
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.owns {
            unsafe { CoUninitialize() };
        }
    }
}

/// Liste les périphériques actifs d'un sens donné.
pub fn list_devices(direction: Direction) -> Result<Vec<AudioDevice>> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

        let default_id = enumerator
            .GetDefaultAudioEndpoint(direction.as_dataflow(), eMultimedia)
            .ok()
            .and_then(|d| d.GetId().ok())
            // Closure explicite : une `unsafe fn` ne peut pas être passée
            // comme valeur à `map`, même depuis un bloc `unsafe`.
            .map(|p| take_pwstr(p));

        let collection =
            enumerator.EnumAudioEndpoints(direction.as_dataflow(), DEVICE_STATE_ACTIVE)?;
        let count = collection.GetCount()?;

        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let device = match collection.Item(i) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let id = match device.GetId() {
                Ok(p) => take_pwstr(p),
                Err(_) => continue,
            };

            // Un périphérique peut refuser d'ouvrir son property store
            // (droits, périphérique fantôme) : on garde l'entrée quand même.
            //
            // windows-core n'expose pas PropVariantToStringAlloc, mais
            // PROPVARIANT sait se convertir en BSTR, qui est déjà libéré
            // automatiquement au Drop.
            let name = device
                .OpenPropertyStore(STGM_READ)
                .ok()
                .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName).ok())
                .and_then(|pv| BSTR::try_from(&pv).ok())
                .map(|b| b.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| String::from("<sans nom>"));

            let is_default = default_id.as_deref() == Some(id.as_str());
            out.push(AudioDevice {
                id,
                name,
                is_default,
            });
        }

        Ok(out)
    }
}

/// Raccourci : les sorties uniquement.
pub fn list_render_devices() -> Result<Vec<AudioDevice>> {
    list_devices(Direction::Render)
}

/// Bascule la sortie par défaut pour les trois rôles.
///
/// `id` doit être un ID renvoyé par `list_devices`, pas un nom.
pub fn set_default_device(id: &str) -> Result<()> {
    policy_config::set_default_endpoint_all_roles(id)
}

/// Consomme un `PWSTR` alloué par COM : le copie en `String` puis le libère.
unsafe fn take_pwstr(p: windows::core::PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const c_void));
    s
}
