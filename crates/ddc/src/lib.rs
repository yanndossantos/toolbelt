//! Contrôle DDC/CI des écrans externes sous Windows.
//!
//! Toutes les fonctions de ce crate sont **bloquantes** et lentes (I2C sur le
//! câble vidéo). Elles doivent tourner sur un thread dédié, jamais sur le
//! thread UI.

pub mod sys;

mod monitor;

pub use monitor::{
    enumerate, DdcError, Monitor, Result, VcpValue, VCP_AUDIO_VOLUME, VCP_BRIGHTNESS, VCP_CONTRAST,
    VCP_INPUT_SOURCE, VCP_POWER_MODE,
};
