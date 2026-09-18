//! Capture d'écran sous Windows.
//!
//! La capture porte toujours sur le **bureau virtuel entier** — le rectangle
//! englobant tous les écrans — et le recadrage se fait ensuite en mémoire.
//!
//! Cet ordre n'est pas un détail. Il garantit que l'overlay de sélection ne
//! peut pas se retrouver dans l'image : les pixels sont figés avant qu'il
//! n'apparaisse. Recadrer d'abord obligerait à capturer après le geste, donc
//! à faire disparaître l'overlay et à espérer que Windows ait fini de
//! repeindre.

use std::fmt;
use std::path::Path;

pub mod sys;

#[cfg(windows)]
mod grab;

#[cfg(windows)]
pub use grab::capture_virtual_screen;

/// Image en mémoire, 3 octets par pixel (RVB), lignes de haut en bas.
///
/// Pas de canal alpha : `BitBlt` depuis le DC écran laisse le quatrième
/// octet à une valeur arbitraire, jamais à une opacité utile. Le garder
/// gonflerait le fichier d'un tiers pour du bruit.
#[derive(Clone)]
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    /// Coin haut-gauche dans les coordonnées écran Windows. Négatif si un
    /// écran est placé à gauche ou au-dessus de l'écran principal.
    pub origin_x: i32,
    pub origin_y: i32,
    pub rgb: Vec<u8>,
}

#[derive(Debug)]
pub enum CaptureError {
    /// Appel Win32 en échec, avec le code de `GetLastError`.
    Win32(&'static str, u32),
    /// Bureau virtuel de taille nulle : aucun écran actif.
    NoScreen,
    /// Rectangle de recadrage entièrement hors de l'image.
    EmptyCrop,
    Io(std::io::Error),
    Png(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Win32(call, code) => write!(f, "{call} a échoué (code {code})"),
            Self::NoScreen => write!(f, "aucun écran actif"),
            Self::EmptyCrop => write!(f, "la sélection est hors de l'écran"),
            Self::Io(e) => write!(f, "écriture impossible : {e}"),
            Self::Png(e) => write!(f, "encodage PNG impossible : {e}"),
        }
    }
}

impl std::error::Error for CaptureError {}

impl From<std::io::Error> for CaptureError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, CaptureError>;

impl Screenshot {
    /// Recadre en coordonnées **écran**, pas en coordonnées image.
    ///
    /// L'appelant raisonne en pixels Windows (ceux de `GetCursorPos`, ceux
    /// d'un `MONITORINFO`), où l'origine peut être négative. Faire la
    /// soustraction ici évite que chaque appelant la refasse, et se trompe.
    ///
    /// Le rectangle est rogné sur les bords de l'image. Une sélection qui
    /// dépasse rend donc moins que demandé plutôt qu'une erreur — sauf si
    /// elle est entièrement dehors, ce qui trahit un bug d'appelant.
    pub fn crop_screen(&self, x: i32, y: i32, width: u32, height: u32) -> Result<Screenshot> {
        let left = x.saturating_sub(self.origin_x).max(0) as i64;
        let top = y.saturating_sub(self.origin_y).max(0) as i64;
        let right = (left + width as i64).min(self.width as i64);
        let bottom = (top + height as i64).min(self.height as i64);

        if left >= right || top >= bottom {
            return Err(CaptureError::EmptyCrop);
        }

        let (w, h) = ((right - left) as u32, (bottom - top) as u32);
        let src_stride = self.width as usize * 3;
        let dst_stride = w as usize * 3;
        let mut rgb = Vec::with_capacity(dst_stride * h as usize);

        for row in 0..h as usize {
            let start = (top as usize + row) * src_stride + left as usize * 3;
            rgb.extend_from_slice(&self.rgb[start..start + dst_stride]);
        }

        Ok(Screenshot {
            width: w,
            height: h,
            origin_x: self.origin_x + left as i32,
            origin_y: self.origin_y + top as i32,
            rgb,
        })
    }

    /// Écrit un PNG. Sans perte : les pixels relus sont identiques à ceux de
    /// l'écran. Le fichier est compressé, mais la compression PNG (deflate)
    /// est réversible — rien de ce qui a été capturé n'est jeté.
    pub fn write_png(&self, path: &Path) -> Result<()> {
        let data = self.encode_png()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn encode_png(&self) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        let mut encoder = png::Encoder::new(&mut data, self.width, self.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);

        let mut writer = encoder
            .write_header()
            .map_err(|e| CaptureError::Png(e.to_string()))?;
        writer
            .write_image_data(&self.rgb)
            .map_err(|e| CaptureError::Png(e.to_string()))?;
        writer
            .finish()
            .map_err(|e| CaptureError::Png(e.to_string()))?;
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Image 4×3 où chaque pixel vaut (x, y, 0). Permet de vérifier qu'un
    /// recadrage rend bien les pixels demandés et pas ceux d'à côté : une
    /// erreur de stride d'un pixel est visible immédiatement.
    fn fixture(origin_x: i32, origin_y: i32) -> Screenshot {
        let (w, h) = (4u32, 3u32);
        let mut rgb = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgb.extend_from_slice(&[x as u8, y as u8, 0]);
            }
        }
        Screenshot {
            width: w,
            height: h,
            origin_x,
            origin_y,
            rgb,
        }
    }

    #[test]
    fn crop_rend_les_pixels_demandes() {
        let shot = fixture(0, 0);
        let out = shot.crop_screen(1, 1, 2, 2).unwrap();
        assert_eq!((out.width, out.height), (2, 2));
        // (1,1) (2,1) / (1,2) (2,2)
        assert_eq!(out.rgb, vec![1, 1, 0, 2, 1, 0, 1, 2, 0, 2, 2, 0]);
    }

    /// Le cas qui casse sur un écran secondaire placé à gauche du principal :
    /// l'origine du bureau virtuel est négative, et une soustraction oubliée
    /// décale toute la capture.
    #[test]
    fn crop_soustrait_une_origine_negative() {
        let shot = fixture(-2, -1);
        let out = shot.crop_screen(-1, 0, 2, 2).unwrap();
        assert_eq!((out.width, out.height), (2, 2));
        assert_eq!(out.rgb, vec![1, 1, 0, 2, 1, 0, 1, 2, 0, 2, 2, 0]);
        assert_eq!((out.origin_x, out.origin_y), (-1, 0));
    }

    #[test]
    fn crop_rogne_sur_les_bords() {
        let shot = fixture(0, 0);
        let out = shot.crop_screen(3, 0, 10, 10).unwrap();
        assert_eq!((out.width, out.height), (1, 3));
    }

    #[test]
    fn crop_entierement_dehors_est_une_erreur() {
        let shot = fixture(0, 0);
        assert!(matches!(
            shot.crop_screen(50, 50, 4, 4),
            Err(CaptureError::EmptyCrop)
        ));
    }
}
