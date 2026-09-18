//! `IPolicyConfig` — interface COM **non documentée** par Microsoft.
//!
//! Il n'existe aucune API publique pour changer le périphérique audio par
//! défaut de Windows. Cette interface est la solution utilisée par tout le
//! monde depuis Vista, EarTrumpet compris. Elle fonctionne, mais rien ne la
//! garantit : c'est le point le plus fragile du projet.
//!
//! ## Point critique : l'ordre de la vtable
//!
//! On ne se sert que de `SetDefaultEndpoint`. Mais en COM, une méthode est
//! identifiée par sa **position** dans la vtable, pas par son nom. Les dix
//! méthodes déclarées avant elle sont donc des bouchons obligatoires : leurs
//! signatures ne comptent pas (on ne les appellera jamais), mais leur
//! **nombre** est structurel.
//!
//! `SetDefaultEndpoint` est la 11e méthode de l'interface, donc l'entrée 13
//! de la vtable (3 entrées d'IUnknown + 10). Ne rien ajouter ni retirer
//! au-dessus d'elle.

#![allow(non_snake_case)]

use std::ffi::c_void;

use windows::core::{Result, GUID, HRESULT, PCWSTR};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows_core::{interface, IUnknown, IUnknown_Vtbl};

/// CLSID_CPolicyConfigClient
pub const CLSID_POLICY_CONFIG_CLIENT: GUID =
    GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// Rôles audio (`ERole` de mmdeviceapi.h).
pub const E_ROLE_CONSOLE: i32 = 0;
pub const E_ROLE_MULTIMEDIA: i32 = 1;
pub const E_ROLE_COMMUNICATIONS: i32 = 2;

/// Les trois rôles. Windows les gère séparément : ne changer que
/// `eMultimedia` donne l'impression que le basculement n'a pas marché
/// quand une appli de visio utilise `eCommunications`.
pub const ALL_ROLES: [i32; 3] = [E_ROLE_CONSOLE, E_ROLE_MULTIMEDIA, E_ROLE_COMMUNICATIONS];

#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
pub unsafe trait IPolicyConfig: IUnknown {
    // --- bouchons : ne pas appeler, ne pas supprimer -------------------
    unsafe fn GetMixFormat(&self, _a: *const c_void, _b: *mut c_void) -> HRESULT;
    unsafe fn GetDeviceFormat(&self, _a: *const c_void, _b: i32, _c: *mut c_void) -> HRESULT;
    unsafe fn ResetDeviceFormat(&self, _a: *const c_void) -> HRESULT;
    unsafe fn SetDeviceFormat(
        &self,
        _a: *const c_void,
        _b: *mut c_void,
        _c: *mut c_void,
    ) -> HRESULT;
    unsafe fn GetProcessingPeriod(
        &self,
        _a: *const c_void,
        _b: i32,
        _c: *mut c_void,
        _d: *mut c_void,
    ) -> HRESULT;
    unsafe fn SetProcessingPeriod(&self, _a: *const c_void, _b: *mut c_void) -> HRESULT;
    unsafe fn GetShareMode(&self, _a: *const c_void, _b: *mut c_void) -> HRESULT;
    unsafe fn SetShareMode(&self, _a: *const c_void, _b: *mut c_void) -> HRESULT;
    unsafe fn GetPropertyValue(
        &self,
        _a: *const c_void,
        _b: i32,
        _c: *const c_void,
        _d: *mut c_void,
    ) -> HRESULT;
    unsafe fn SetPropertyValue(
        &self,
        _a: *const c_void,
        _b: i32,
        _c: *const c_void,
        _d: *const c_void,
    ) -> HRESULT;
    // --- fin des bouchons ----------------------------------------------

    /// Bascule le périphérique par défaut pour un rôle donné.
    /// `device_id` est l'ID renvoyé par `IMMDevice::GetId`.
    unsafe fn SetDefaultEndpoint(&self, device_id: PCWSTR, role: i32) -> HRESULT;

    unsafe fn SetEndpointVisibility(&self, device_id: PCWSTR, visible: i32) -> HRESULT;
}

/// Bascule la sortie par défaut pour les trois rôles.
///
/// Cette fonction vit ici et pas dans `lib.rs` pour une raison de visibilité :
/// la macro `#[interface]` génère des méthodes privées au module où le trait
/// est déclaré. L'appel doit donc se faire dans ce fichier.
pub fn set_default_endpoint_all_roles(device_id: &str) -> Result<()> {
    let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let policy: IPolicyConfig =
            CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)?;

        for role in ALL_ROLES {
            policy
                .SetDefaultEndpoint(PCWSTR(wide.as_ptr()), role)
                .ok()?;
        }
    }
    Ok(())
}
