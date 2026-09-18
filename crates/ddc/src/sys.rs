//! Déclarations FFI brutes.
//!
//! Tout est écrit à la main plutôt que repris de windows-rs, pour deux raisons :
//! il n'y a que 8 fonctions, et les signatures de windows-rs pour Dxva2 changent
//! d'une version mineure à l'autre (parfois `Result<()>`, parfois `i32`).
//!
//! Toutes ces fonctions renvoient un `BOOL` Win32 : 0 = échec, il faut alors
//! appeler `GetLastError()`.

#![allow(non_snake_case, non_camel_case_types)]

use std::ffi::c_void;

pub type BOOL = i32;
pub type DWORD = u32;
pub type HANDLE = *mut c_void;
pub type HMONITOR = *mut c_void;
pub type HDC = *mut c_void;
pub type LPARAM = isize;

/// PHYSICAL_MONITOR_DESCRIPTION_SIZE (physicalmonitorenumerationapi.h)
pub const PHYSICAL_MONITOR_DESCRIPTION_SIZE: usize = 128;
/// CCHDEVICENAME (wingdi.h)
pub const CCHDEVICENAME: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MONITORINFOEXW {
    pub cbSize: DWORD,
    pub rcMonitor: RECT,
    pub rcWork: RECT,
    pub dwFlags: DWORD,
    pub szDevice: [u16; CCHDEVICENAME],
}

impl Default for MONITORINFOEXW {
    fn default() -> Self {
        Self {
            // GetMonitorInfoW distingue MONITORINFO de MONITORINFOEXW
            // uniquement par ce champ. S'il est faux, l'appel échoue.
            cbSize: std::mem::size_of::<MONITORINFOEXW>() as DWORD,
            rcMonitor: RECT::default(),
            rcWork: RECT::default(),
            dwFlags: 0,
            szDevice: [0; CCHDEVICENAME],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PHYSICAL_MONITOR {
    pub hPhysicalMonitor: HANDLE,
    pub szPhysicalMonitorDescription: [u16; PHYSICAL_MONITOR_DESCRIPTION_SIZE],
}

impl Default for PHYSICAL_MONITOR {
    fn default() -> Self {
        Self {
            hPhysicalMonitor: std::ptr::null_mut(),
            szPhysicalMonitorDescription: [0; PHYSICAL_MONITOR_DESCRIPTION_SIZE],
        }
    }
}

pub type MONITORENUMPROC = unsafe extern "system" fn(HMONITOR, HDC, *mut RECT, LPARAM) -> BOOL;

#[link(name = "user32")]
extern "system" {
    pub fn EnumDisplayMonitors(
        hdc: HDC,
        lprcClip: *const RECT,
        lpfnEnum: MONITORENUMPROC,
        dwData: LPARAM,
    ) -> BOOL;

    pub fn GetMonitorInfoW(hMonitor: HMONITOR, lpmi: *mut MONITORINFOEXW) -> BOOL;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetLastError() -> DWORD;
}

#[link(name = "Dxva2")]
extern "system" {
    pub fn GetNumberOfPhysicalMonitorsFromHMONITOR(
        hMonitor: HMONITOR,
        pdwNumberOfPhysicalMonitors: *mut DWORD,
    ) -> BOOL;

    pub fn GetPhysicalMonitorsFromHMONITOR(
        hMonitor: HMONITOR,
        dwPhysicalMonitorArraySize: DWORD,
        pPhysicalMonitorArray: *mut PHYSICAL_MONITOR,
    ) -> BOOL;

    pub fn DestroyPhysicalMonitors(
        dwPhysicalMonitorArraySize: DWORD,
        pPhysicalMonitorArray: *mut PHYSICAL_MONITOR,
    ) -> BOOL;

    pub fn GetVCPFeatureAndVCPFeatureReply(
        hMonitor: HANDLE,
        bVCPCode: u8,
        pvct: *mut u32,
        pdwCurrentValue: *mut DWORD,
        pdwMaximumValue: *mut DWORD,
    ) -> BOOL;

    pub fn SetVCPFeature(hMonitor: HANDLE, bVCPCode: u8, dwNewValue: DWORD) -> BOOL;

    pub fn GetCapabilitiesStringLength(
        hMonitor: HANDLE,
        pdwCapabilitiesStringLengthInCharacters: *mut DWORD,
    ) -> BOOL;

    pub fn CapabilitiesRequestAndCapabilitiesReply(
        hMonitor: HANDLE,
        pszASCIICapabilitiesString: *mut u8,
        dwCapabilitiesStringLengthInCharacters: DWORD,
    ) -> BOOL;
}

/// Convertit un buffer UTF-16 terminé par NUL (ou plein) en `String`.
pub fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
