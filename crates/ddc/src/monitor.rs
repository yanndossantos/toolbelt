use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::thread::sleep;
use std::time::Duration;

use crate::sys::*;

// --- Codes VCP courants (norme MCCS) ------------------------------------

pub const VCP_BRIGHTNESS: u8 = 0x10;
pub const VCP_CONTRAST: u8 = 0x12;
pub const VCP_INPUT_SOURCE: u8 = 0x60;
pub const VCP_AUDIO_VOLUME: u8 = 0x62;
pub const VCP_POWER_MODE: u8 = 0xD6;

// --- Erreurs ------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum DdcError {
    /// EnumDisplayMonitors a échoué.
    Enumeration(u32),
    /// Lecture d'un code VCP refusée par l'écran.
    VcpRead { code: u8, last_error: u32 },
    /// Écriture d'un code VCP refusée par l'écran.
    VcpWrite { code: u8, last_error: u32 },
    /// L'écran n'a pas renvoyé sa chaîne de capacités.
    Capabilities(u32),
}

impl fmt::Display for DdcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DdcError::Enumeration(e) => {
                write!(f, "énumération des écrans impossible (GetLastError={e})")
            }
            DdcError::VcpRead { code, last_error } => write!(
                f,
                "lecture VCP 0x{code:02X} refusée (GetLastError={last_error})"
            ),
            DdcError::VcpWrite { code, last_error } => write!(
                f,
                "écriture VCP 0x{code:02X} refusée (GetLastError={last_error})"
            ),
            DdcError::Capabilities(e) => {
                write!(f, "chaîne de capacités indisponible (GetLastError={e})")
            }
        }
    }
}

impl std::error::Error for DdcError {}

pub type Result<T> = std::result::Result<T, DdcError>;

// --- Valeur VCP ---------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpValue {
    pub current: u32,
    pub max: u32,
}

impl VcpValue {
    /// Valeur ramenée sur 0..=100. Beaucoup d'écrans ont déjà max=100,
    /// mais pas tous : certains montent à 255, d'autres à 1000.
    pub fn percent(&self) -> u32 {
        if self.max == 0 {
            return 0;
        }
        ((self.current as u64 * 100) / self.max as u64) as u32
    }
}

// --- Écran physique -----------------------------------------------------

/// Un écran physique joignable en DDC/CI.
///
/// Le handle est libéré automatiquement (`DestroyPhysicalMonitors`) au Drop.
/// Il devient invalide au réveil de veille, au hotplug et lors d'une mise à
/// jour du pilote GPU : il faut alors jeter la liste et rappeler `enumerate()`.
pub struct Monitor {
    raw: PHYSICAL_MONITOR,
    /// Nom d'adaptateur GDI, ex. `\\.\DISPLAY1`.
    pub adapter: String,
    /// Description renvoyée par le pilote. Souvent générique
    /// ("Generic PnP Monitor") — voir la note dans le README.
    pub description: String,
    /// Index de cet écran physique sur son HMONITOR (>0 seulement si
    /// plusieurs écrans partagent une sortie, cas rare).
    pub index_on_adapter: usize,
}

// Le handle DDC peut être utilisé depuis un autre thread que celui qui l'a
// créé, tant qu'un seul thread s'en sert à la fois. D'où Send mais pas Sync.
unsafe impl Send for Monitor {}

impl Drop for Monitor {
    fn drop(&mut self) {
        unsafe {
            DestroyPhysicalMonitors(1, &mut self.raw);
        }
    }
}

impl fmt::Debug for Monitor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Monitor")
            .field("adapter", &self.adapter)
            .field("description", &self.description)
            .finish()
    }
}

impl Monitor {
    /// Lit un code VCP. Un seul essai.
    ///
    /// Attention : appel I2C bloquant, typiquement 20 à 300 ms, parfois bien
    /// plus. Jamais depuis le thread UI.
    pub fn get_vcp(&self, code: u8) -> Result<VcpValue> {
        let mut vct: u32 = 0;
        let mut current: DWORD = 0;
        let mut max: DWORD = 0;

        let ok = unsafe {
            GetVCPFeatureAndVCPFeatureReply(
                self.raw.hPhysicalMonitor,
                code,
                &mut vct,
                &mut current,
                &mut max,
            )
        };

        if ok == 0 {
            return Err(DdcError::VcpRead {
                code,
                last_error: unsafe { GetLastError() },
            });
        }
        Ok(VcpValue { current, max })
    }

    /// Écrit un code VCP. Un seul essai. Même remarque sur le blocage.
    pub fn set_vcp(&self, code: u8, value: u32) -> Result<()> {
        let ok = unsafe { SetVCPFeature(self.raw.hPhysicalMonitor, code, value) };
        if ok == 0 {
            return Err(DdcError::VcpWrite {
                code,
                last_error: unsafe { GetLastError() },
            });
        }
        Ok(())
    }

    /// Lecture avec réessais. Le DDC/CI est intrinsèquement peu fiable :
    /// un échec isolé ne veut pas dire que l'écran ne supporte pas le code.
    pub fn get_vcp_retry(&self, code: u8, attempts: u32, backoff: Duration) -> Result<VcpValue> {
        let mut last = DdcError::VcpRead {
            code,
            last_error: 0,
        };
        for i in 0..attempts.max(1) {
            match self.get_vcp(code) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last = e;
                    if i + 1 < attempts {
                        sleep(backoff);
                    }
                }
            }
        }
        Err(last)
    }

    /// Écriture avec réessais.
    pub fn set_vcp_retry(
        &self,
        code: u8,
        value: u32,
        attempts: u32,
        backoff: Duration,
    ) -> Result<()> {
        let mut last = DdcError::VcpWrite {
            code,
            last_error: 0,
        };
        for i in 0..attempts.max(1) {
            match self.set_vcp(code, value) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last = e;
                    if i + 1 < attempts {
                        sleep(backoff);
                    }
                }
            }
        }
        Err(last)
    }

    pub fn brightness(&self) -> Result<VcpValue> {
        self.get_vcp_retry(VCP_BRIGHTNESS, 3, Duration::from_millis(40))
    }

    /// `percent` est ramené dans 0..=100 puis remis à l'échelle du `max`
    /// de l'écran, qui n'est pas forcément 100.
    pub fn set_brightness_percent(&self, percent: u32) -> Result<()> {
        let max = self.brightness()?.max.max(1);
        let p = percent.min(100) as u64;
        let value = ((p * max as u64) / 100) as u32;
        self.set_vcp_retry(VCP_BRIGHTNESS, value, 3, Duration::from_millis(40))
    }

    /// Chaîne de capacités MCCS brute. C'est elle qui dit quels codes VCP
    /// l'écran prétend supporter (section `vcp(...)`).
    pub fn capabilities(&self) -> Result<String> {
        let mut len: DWORD = 0;
        let ok = unsafe { GetCapabilitiesStringLength(self.raw.hPhysicalMonitor, &mut len) };
        if ok == 0 || len == 0 {
            return Err(DdcError::Capabilities(unsafe { GetLastError() }));
        }

        // len inclut le NUL final.
        let mut buf = vec![0u8; len as usize];
        let ok = unsafe {
            CapabilitiesRequestAndCapabilitiesReply(
                self.raw.hPhysicalMonitor,
                buf.as_mut_ptr(),
                len,
            )
        };
        if ok == 0 {
            return Err(DdcError::Capabilities(unsafe { GetLastError() }));
        }

        if let Some(pos) = buf.iter().position(|&b| b == 0) {
            buf.truncate(pos);
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

// --- Énumération --------------------------------------------------------

struct EnumCtx {
    found: Vec<(HMONITOR, String)>,
}

unsafe extern "system" fn enum_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _rc: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // Un panic qui traverse une frontière FFI est un comportement indéfini.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let ctx = &mut *(data as *mut EnumCtx);
        let mut info = MONITORINFOEXW::default();
        let name = if GetMonitorInfoW(hmon, &mut info) != 0 {
            wide_to_string(&info.szDevice)
        } else {
            String::from("<inconnu>")
        };
        ctx.found.push((hmon, name));
    }));
    1 // continuer l'énumération
}

/// Énumère tous les écrans physiques joignables en DDC/CI.
///
/// Les écrans qui n'exposent pas de moniteur physique (RDP, certains écrans
/// virtuels, dalles de portable sans DDC) sont silencieusement ignorés.
pub fn enumerate() -> Result<Vec<Monitor>> {
    let mut ctx = EnumCtx { found: Vec::new() };

    let ok = unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            enum_proc,
            &mut ctx as *mut EnumCtx as LPARAM,
        )
    };
    if ok == 0 {
        return Err(DdcError::Enumeration(unsafe { GetLastError() }));
    }

    let mut out = Vec::new();
    for (hmon, adapter) in ctx.found {
        let mut count: DWORD = 0;
        let ok = unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut count) };
        if ok == 0 || count == 0 {
            continue;
        }

        let mut arr = vec![PHYSICAL_MONITOR::default(); count as usize];
        let ok = unsafe { GetPhysicalMonitorsFromHMONITOR(hmon, count, arr.as_mut_ptr()) };
        if ok == 0 {
            continue;
        }

        // À partir d'ici chaque handle est possédé par un Monitor, qui le
        // libérera au Drop. Ne pas appeler DestroyPhysicalMonitors sur `arr`.
        for (i, pm) in arr.into_iter().enumerate() {
            out.push(Monitor {
                description: wide_to_string(&pm.szPhysicalMonitorDescription),
                raw: pm,
                adapter: adapter.clone(),
                index_on_adapter: i,
            });
        }
    }

    Ok(out)
}
