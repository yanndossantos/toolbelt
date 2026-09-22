#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod ddc_worker;
mod hotkey;
mod screenshot;

use base64::Engine;
use arboard::{Clipboard, ImageData};
use ddc_worker::{DdcWorker, MonitorInfo};
use screenshot::{HotkeyError, Pending};
use std::borrow::Cow;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use tauri::{Manager, Rect, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use toolbelt_audio as audio;

const FLYOUT: &str = "flyout";
const OVERLAY: &str = "overlay";
const TOAST: &str = "toast";
const TRAY: &str = "main";
/// Taille de la confirmation, en pixels logiques. Fixe : la fenêtre est
/// assez grande pour un chemin de longueur courante, et le texte se replie
/// dedans plutôt que de faire respirer la fenêtre à chaque capture.
///
/// Ces dimensions incluent `TOAST_SHADOW` de chaque côté : la carte visible
/// fait 404 × 84. La largeur tient compte du bouton « Ouvrir », qui prend sa
/// place à droite du texte — trop étroite, le chemin passerait sur deux
/// lignes et déborderait de la hauteur.
const TOAST_W: f64 = 440.0;
const TOAST_H: f64 = 120.0;
/// Bordure transparente réservée à l'ombre portée de la carte, à tenir égale
/// à la marge de `#card` dans `toast.html`. Une fenêtre plus serrée que
/// l'ombre la trancherait net sur ses bords.
const TOAST_SHADOW: f64 = 18.0;
/// Écart entre la carte **visible** et le bord de l'écran, et hauteur
/// réservée à la barre des tâches.
const TOAST_MARGIN: f64 = 16.0;
const TASKBAR: f64 = 60.0;
const FLYOUT_W: f64 = 360.0;
/// Hauteur de départ. La fenêtre se redimensionne ensuite sur son contenu.
const FLYOUT_H: f64 = 420.0;
/// Bornes du redimensionnement automatique, en pixels logiques.
const FLYOUT_MIN_H: f64 = 200.0;
const FLYOUT_MAX_H: f64 = 1100.0;
/// Écart entre le bas du flyout et le haut de l'icône de la zone de notification.
const GAP: f64 = 12.0;

// --- Commandes exposées au frontend ------------------------------------

#[tauri::command]
fn list_monitors(worker: tauri::State<DdcWorker>) -> Vec<MonitorInfo> {
    worker.snapshot()
}

#[tauri::command]
fn set_brightness(worker: tauri::State<DdcWorker>, index: usize, percent: u32) {
    worker.set_brightness(index, percent);
}

#[tauri::command]
fn reload_monitors(worker: tauri::State<DdcWorker>) {
    worker.reload();
}

#[tauri::command]
fn list_audio() -> Result<Vec<audio::AudioDevice>, String> {
    let _com = audio::ComGuard::new().map_err(|e| e.to_string())?;
    audio::list_render_devices().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_audio(app: tauri::AppHandle, id: String) -> Result<(), String> {
    {
        let _com = audio::ComGuard::new().map_err(|e| e.to_string())?;
        audio::set_default_device(&id).map_err(|e| e.to_string())?;
    }
    // La coche du menu du tray désigne la sortie par défaut : elle vient de
    // changer.
    refresh_tray_menu(&app, true);
    Ok(())
}

// --- Mélangeur ---------------------------------------------------------
//
// Contrairement au DDC, ces appels sont locaux et rapides (bien moins d'une
// milliseconde). Pas besoin de worker ni de bridage : on les exécute
// directement sur le thread de la commande.

#[tauri::command]
fn list_sessions() -> Result<Vec<audio::sessions::AudioSession>, String> {
    let _com = audio::ComGuard::new().map_err(|e| e.to_string())?;
    audio::sessions::list_sessions().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_session_volume(pid: u32, volume: f32) -> Result<(), String> {
    let _com = audio::ComGuard::new().map_err(|e| e.to_string())?;
    audio::sessions::set_session_volume(pid, volume).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_session_mute(pid: u32, muted: bool) -> Result<(), String> {
    let _com = audio::ComGuard::new().map_err(|e| e.to_string())?;
    audio::sessions::set_session_mute(pid, muted).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_icon(pid: u32) -> Option<audio::icons::IconRgba> {
    audio::icons::process_icon(pid)
}

#[tauri::command]
fn get_master() -> Result<audio::sessions::MasterState, String> {
    let _com = audio::ComGuard::new().map_err(|e| e.to_string())?;
    audio::sessions::master_state().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_master_volume(volume: f32) -> Result<(), String> {
    let _com = audio::ComGuard::new().map_err(|e| e.to_string())?;
    audio::sessions::set_master_volume(volume).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_master_mute(muted: bool) -> Result<(), String> {
    let _com = audio::ComGuard::new().map_err(|e| e.to_string())?;
    audio::sessions::set_master_mute(muted).map_err(|e| e.to_string())
}

/// État du démarrage automatique, tel qu'affiché dans le pied du flyout.
#[derive(serde::Serialize)]
struct AutostartState {
    enabled: bool,
    /// Chemin qui serait enregistré. Affiché en infobulle pour que l'on voie
    /// tout de suite si c'est un binaire de debug dans Downloads.
    command: String,
    /// Valeur déjà présente si elle diffère de la nôtre.
    other: Option<String>,
    error: Option<String>,
}

/// La case n'est cochée que si l'entrée existe **et** pointe sur cet
/// exécutable-ci. La distinction compte : une entrée laissée par une
/// ancienne copie est pire que pas d'entrée du tout, parce qu'elle lancerait
/// un binaire périmé ou disparu.
#[tauri::command]
fn get_autostart() -> AutostartState {
    let command = autostart::current_command().unwrap_or_default();

    match autostart::registered_command() {
        Ok(registered) => {
            let enabled = registered
                .as_deref()
                .map(|r| r.eq_ignore_ascii_case(&command))
                .unwrap_or(false);
            AutostartState {
                enabled,
                command,
                other: registered.filter(|_| !enabled),
                error: None,
            }
        }
        Err(e) => AutostartState {
            enabled: false,
            command,
            other: None,
            error: Some(e),
        },
    }
}

#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    autostart::set(enabled)
}

#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

// --- Capture d'écran ----------------------------------------------------

/// Fige l'écran et ouvre l'overlay de sélection.
///
/// Appelé depuis le thread du raccourci, jamais depuis l'UI. La capture y
/// est faite directement — 80 ms sur deux écrans 1440p — plutôt que d'être
/// renvoyée au thread principal : la retarder laisserait le temps à
/// l'utilisateur de relâcher les touches devant une fenêtre qui change.
fn begin_capture(app: &tauri::AppHandle) {
    let shot = match toolbelt_capture::capture_virtual_screen() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("capture : {e}");
            return;
        }
    };

    let (x, y) = (shot.origin_x, shot.origin_y);
    let (w, h) = (shot.width, shot.height);

    if let Some(pending) = app.try_state::<Pending>() {
        if let Ok(mut guard) = pending.0.lock() {
            *guard = Some(shot);
        }
    }

    let handle = app.clone();
    // Déplacer et afficher une fenêtre sont des opérations Win32 : elles
    // doivent partir du thread qui possède la boucle de messages de l'UI.
    let _ = app.run_on_main_thread(move || {
        let Some(win) = handle.get_webview_window(OVERLAY) else {
            return;
        };
        // En pixels physiques : l'overlay doit recouvrir le bureau virtuel
        // exactement, sinon la correspondance entre le clic et l'image
        // capturée est décalée.
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
        let _ = win.set_size(tauri::PhysicalSize::new(w, h));
        let _ = win.show();
        let _ = win.set_focus();
    });
}

fn close_overlay(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window(OVERLAY) {
        let _ = win.hide();
    }
}

/// Contenu de la confirmation. Le nom du fichier et son dossier sont séparés
/// pour que le frontend puisse mettre en avant le premier : c'est la partie
/// qu'on cherche à lire, le dossier est toujours le même.
#[derive(Clone, serde::Serialize)]
struct SavedToast {
    name: String,
    dir: String,
}

/// Affiche la confirmation en bas à droite de l'écran principal.
///
/// Fenêtre séparée, et pas un message dans l'overlay : l'overlay couvre le
/// bureau virtuel entier et bloque tout tant qu'il est là. La confirmation
/// doit pouvoir rester à l'écran pendant qu'on retourne travailler, donc
/// elle ne prend pas le focus (`never_activate`) et laisse passer les clics
/// (`set_ignore_cursor_events`).
fn show_toast(app: &tauri::AppHandle, path: &std::path::Path) {
    let payload = SavedToast {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        dir: path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    };

    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(win) = handle.get_webview_window(TOAST) else {
            return;
        };

        // Coin bas-droit de l'écran principal, au-dessus de la barre des
        // tâches. Même repli que le flyout quand il n'a pas d'ancre.
        //
        // `TOAST_SHADOW` est retranché des deux écarts : la fenêtre déborde
        // de cette bordure transparente sur chaque côté, et c'est la carte
        // visible, pas la fenêtre, qu'on veut voir à `TOAST_MARGIN` du bord.
        if let Ok(Some(monitor)) = win.primary_monitor() {
            let scale = monitor.scale_factor();
            let ms = monitor.size();
            let mp = monitor.position();
            let x = mp.x as f64 + ms.width as f64 - (TOAST_W + TOAST_MARGIN - TOAST_SHADOW) * scale;
            let y = mp.y as f64 + ms.height as f64 - (TOAST_H + TASKBAR - TOAST_SHADOW) * scale;
            let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
        }

        // Poussé par `eval`, pas par le système d'événements : `listen` n'a
        // jamais rappelé ici — ni ciblé par label, ni en émission globale.
        // Ce projet ne déclare aucune capacité ACL, ce qui n'empêche pas
        // `invoke` de nos propres commandes mais coupe l'API d'événements.
        // Le symptôme était vicieux : confirmation affichée mais vide, et
        // comme c'est l'écouteur qui arme le minuteur, elle ne s'effaçait
        // plus jamais. `eval` ne dépend d'aucune permission.
        //
        // `to_string` échappe le chemin : un antislash ou un guillemet dans
        // un nom de dossier casserait le littéral JavaScript.
        match serde_json::to_string(&payload) {
            Ok(json) => {
                let _ = win.eval(&format!("window.showSaved({json})"));
            }
            Err(e) => eprintln!("confirmation : {e}"),
        }
        let _ = win.show();
    });
}

/// Referme la confirmation. Appelée par sa propre page après le délai.
#[tauri::command]
fn hide_toast(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window(TOAST) {
        let _ = win.hide();
    }
}

/// Ferme l'outil et libère le buffer.
///
/// Utilisée par les trois sorties : Échap, la croix, et la fermeture
/// automatique deux secondes après un enregistrement réussi.
#[tauri::command]
fn close_capture(app: tauri::AppHandle, pending: tauri::State<Pending>) {
    if let Ok(mut guard) = pending.0.lock() {
        *guard = None;
    }
    close_overlay(&app);
}

#[derive(serde::Serialize)]
struct HoveredWindow {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[tauri::command]
fn get_hovered_window(
    app: tauri::AppHandle,
    pending: tauri::State<Pending>,
) -> Option<HoveredWindow> {
    let guard = pending.0.lock().ok()?;
    let shot = guard.as_ref()?;
    let overlay = app.get_webview_window(OVERLAY)?;
    let overlay_hwnd = overlay.hwnd().ok()?.0 as *mut std::ffi::c_void;
    let scale = overlay.scale_factor().ok()?;
    let (left, top, width, height) = screenshot::hovered_window(overlay_hwnd)?;
    Some(HoveredWindow {
        x: (left - shot.origin_x) as f64 / scale,
        y: (top - shot.origin_y) as f64 / scale,
        w: width as f64 / scale,
        h: height as f64 / scale,
    })
}

/// Recadre la capture figée et l'écrit sur le disque.
///
/// Les coordonnées viennent du frontend en pixels CSS, relatives à
/// l'overlay. Le facteur d'échelle les ramène en pixels physiques, et
/// l'origine du bureau virtuel en coordonnées écran.
///
/// Tant que l'écriture n'a pas réussi, l'image figée est seulement
/// **empruntée** : un échec laisse le rectangle ajustable et de quoi
/// réessayer. La consommer rendrait l'échec définitif — l'outil resterait
/// ouvert sur une erreur devant une sélection devenue inutilisable.
///
/// En cas de succès, tout se referme ici : buffer libéré, overlay masqué, et
/// la confirmation part dans sa propre fenêtre. L'overlay couvre le bureau
/// virtuel entier ; l'y laisser le temps de lire un message bloquerait la
/// machine pour rien.
#[tauri::command]
fn save_capture(
    app: tauri::AppHandle,
    pending: tauri::State<Pending>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<String, String> {
    let path = {
        let guard = pending
            .0
            .lock()
            .map_err(|_| "état de capture corrompu".to_string())?;
        let shot = guard.as_ref().ok_or("aucune capture en attente")?;

        let win = app
            .get_webview_window(OVERLAY)
            .ok_or("overlay introuvable")?;
        let scale = win.scale_factor().unwrap_or(1.0);

        let cropped = shot
            .crop_screen(
                shot.origin_x + (x * scale).round() as i32,
                shot.origin_y + (y * scale).round() as i32,
                (w * scale).round().max(1.0) as u32,
                (h * scale).round().max(1.0) as u32,
            )
            .map_err(|e| e.to_string())?;

        let path = screenshot::output_dir()
            .ok_or("USERPROFILE introuvable")?
            .join(screenshot::file_name());

        cropped.write_png(&path).map_err(|e| e.to_string())?;
        path
    };

    if let Ok(mut guard) = pending.0.lock() {
        *guard = None;
    }
    close_overlay(&app);
    show_toast(&app, &path);

    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn get_capture_preview(
    app: tauri::AppHandle,
    pending: tauri::State<Pending>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<String, String> {
    let guard = pending
        .0
        .lock()
        .map_err(|_| "état de capture corrompu".to_string())?;
    let shot = guard.as_ref().ok_or("aucune capture en attente")?;
    let win = app
        .get_webview_window(OVERLAY)
        .ok_or("overlay introuvable")?;
    let scale = win.scale_factor().unwrap_or(1.0);
    let cropped = shot
        .crop_screen(
            shot.origin_x + (x * scale).round() as i32,
            shot.origin_y + (y * scale).round() as i32,
            (w * scale).round().max(1.0) as u32,
            (h * scale).round().max(1.0) as u32,
        )
        .map_err(|e| e.to_string())?;
    let encoded = cropped.encode_png().map_err(|e| e.to_string())?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(encoded)
    ))
}

#[tauri::command]
fn save_annotated_capture(
    app: tauri::AppHandle,
    pending: tauri::State<Pending>,
    png_data: String,
) -> Result<String, String> {
    persist_annotated_capture(&app, &pending, &png_data, false)
}

#[tauri::command]
fn copy_annotated_capture(
    app: tauri::AppHandle,
    pending: tauri::State<Pending>,
    png_data: String,
) -> Result<String, String> {
    persist_annotated_capture(&app, &pending, &png_data, true)
}

fn persist_annotated_capture(
    app: &tauri::AppHandle,
    pending: &tauri::State<Pending>,
    png_data: &str,
    copy_to_clipboard: bool,
) -> Result<String, String> {
    let encoded = png_data
        .strip_prefix("data:image/png;base64,")
        .ok_or("format PNG invalide")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "données PNG invalides")?;
    if bytes.len() > 100 * 1024 * 1024 {
        return Err("image trop volumineuse".to_string());
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| "données PNG invalides")?;
    let mut decoded = vec![0; reader.output_buffer_size()];
    let output = reader
        .next_frame(&mut decoded)
        .map_err(|_| "données PNG invalides")?;
    let rgba = match output.color_type {
        png::ColorType::Rgba => decoded[..output.buffer_size()].to_vec(),
        png::ColorType::Rgb => decoded[..output.buffer_size()]
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        _ => return Err("format PNG non pris en charge".to_string()),
    };

    let path = screenshot::output_dir()
        .ok_or("USERPROFILE introuvable")?
        .join(screenshot::file_name());
    std::fs::create_dir_all(path.parent().ok_or("dossier invalide")?).map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;

    if copy_to_clipboard {
        let mut clipboard = Clipboard::new().map_err(|e| format!("presse-papiers : {e}"))?;
        clipboard
            .set_image(ImageData {
                width: output.width as usize,
                height: output.height as usize,
                bytes: Cow::Owned(rgba),
            })
            .map_err(|e| format!("presse-papiers : {e}"))?;
    }

    if let Ok(mut guard) = pending.0.lock() {
        *guard = None;
    }
    close_overlay(&app);
    show_toast(&app, &path);
    Ok(path.to_string_lossy().into_owned())
}

/// Message d'erreur si le raccourci n'a pas pu être enregistré, pour le pied
/// du flyout. `None` quand tout va bien.
#[tauri::command]
fn get_hotkey_error(state: tauri::State<HotkeyError>) -> Option<String> {
    state.0.lock().ok().and_then(|g| g.clone())
}

/// Libellé du raccourci, affiché à côté d'un éventuel message d'erreur.
#[tauri::command]
fn get_hotkey_label() -> &'static str {
    hotkey::LABEL
}

/// Ouvre le dossier des captures dans l'Explorateur.
///
/// Le dossier est créé s'il n'existe pas : au premier lancement, aucune
/// capture n'a encore été prise, et ouvrir un chemin absent afficherait une
/// erreur de l'Explorateur au lieu d'un dossier vide.
#[tauri::command]
fn open_screenshots_dir() -> Result<(), String> {
    let dir = screenshot::output_dir().ok_or("USERPROFILE introuvable")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // `explorer.exe` rend un code de sortie non nul même quand il réussit :
    // on lance sans attendre, et seul l'échec du démarrage est une erreur.
    std::process::Command::new("explorer")
        .arg(&dir)
        .spawn()
        .map_err(|e| format!("ouverture de l'Explorateur : {e}"))?;
    Ok(())
}

// --- Positionnement du flyout ------------------------------------------

/// Dernière position connue de l'icône du tray, en pixels physiques :
/// (x, y, largeur, hauteur).
///
/// On la mémorise parce que le flyout se redimensionne après coup, quand le
/// frontend a mesuré son contenu. Sans elle, impossible de le replacer en
/// gardant son bord bas collé au-dessus de l'icône.
#[derive(Default)]
struct TrayAnchor(Mutex<Option<(f64, f64, f64, f64)>>);

/// Colle le bord bas du flyout au-dessus de l'icône du tray.
fn position_flyout(win: &tauri::WebviewWindow, anchor: &TrayAnchor) {
    let size = match win.outer_size() {
        Ok(s) => s,
        Err(_) => return,
    };
    let scale = win.scale_factor().unwrap_or(1.0);

    let stored = match anchor.0.lock() {
        Ok(g) => *g,
        Err(_) => None,
    };

    let (mut x, y) = match stored {
        Some((ax, ay, aw, _ah)) => (
            ax + aw / 2.0 - size.width as f64 / 2.0,
            ay - size.height as f64 - GAP * scale,
        ),
        // Ouverture depuis le menu sans avoir jamais cliqué l'icône : on n'a
        // pas d'ancre. Repli sur le coin bas-droit de l'écran principal,
        // faute de mieux.
        None => match win.primary_monitor() {
            Ok(Some(monitor)) => {
                let ms = monitor.size();
                let mp = monitor.position();
                (
                    mp.x as f64 + ms.width as f64 - size.width as f64 - 12.0 * scale,
                    mp.y as f64 + ms.height as f64 - size.height as f64 - 60.0 * scale,
                )
            }
            _ => return,
        },
    };

    // L'ancre peut être proche du bord droit (et le panneau est plus large
    // qu'une icône de tray). Le centrage seul laisserait alors la fenêtre
    // sortir du moniteur. Utiliser le moniteur qui contient l'ancre évite
    // aussi de déplacer le flyout sur un autre écran dans une configuration
    // multi-moniteurs.
    let monitor = stored
        .and_then(|(ax, ay, aw, ah)| {
            win.available_monitors().ok()?.into_iter().find(|monitor| {
                let position = monitor.position();
                let monitor_size = monitor.size();
                let center_x = ax + aw / 2.0;
                let center_y = ay + ah / 2.0;
                center_x >= position.x as f64
                    && center_x < position.x as f64 + monitor_size.width as f64
                    && center_y >= position.y as f64
                    && center_y < position.y as f64 + monitor_size.height as f64
            })
        })
        .or_else(|| win.primary_monitor().ok().flatten());

    if let Some(monitor) = monitor {
        let position = monitor.position();
        let monitor_size = monitor.size();
        let left = position.x as f64;
        let right = left + monitor_size.width as f64 - size.width as f64;
        x = x.clamp(left, right.max(left));
    }

    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
}

/// Ajuste la hauteur de la fenêtre au contenu mesuré par le frontend.
///
/// C'est ce qui remplace la barre de défilement : le flyout est aussi haut
/// qu'il a besoin de l'être, et pas plus.
#[tauri::command]
fn set_flyout_height(app: tauri::AppHandle, anchor: tauri::State<TrayAnchor>, height: f64) {
    let Some(win) = app.get_webview_window(FLYOUT) else {
        return;
    };

    let clamped = height.clamp(FLYOUT_MIN_H, FLYOUT_MAX_H);
    let _ = win.set_size(tauri::LogicalSize::new(FLYOUT_W, clamped));
    position_flyout(&win, &anchor);
}

/// Mémorise la position de l'icône du tray, place le flyout et l'affiche.
fn show_flyout(win: &tauri::WebviewWindow, anchor: &TrayAnchor, tray_rect: Rect) {
    let scale = win.scale_factor().unwrap_or(1.0);
    let pos = tray_rect.position.to_physical::<f64>(scale);
    let size = tray_rect.size.to_physical::<f64>(scale);

    if let Ok(mut guard) = anchor.0.lock() {
        *guard = Some((pos.x, pos.y, size.width, size.height));
    }

    position_flyout(win, anchor);
    let _ = win.show();
    let _ = win.set_focus();
}

fn toggle_flyout(win: &tauri::WebviewWindow, anchor: &TrayAnchor, tray_rect: Rect) {
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        show_flyout(win, anchor, tray_rect);
    }
}

// --- Menu du tray -------------------------------------------------------

/// Préfixe des identifiants d'entrées de sortie audio.
const AUDIO_ITEM: &str = "audio:";

fn audio_outputs() -> Vec<audio::AudioDevice> {
    let Ok(_com) = audio::ComGuard::new() else {
        return Vec::new();
    };
    audio::list_render_devices().unwrap_or_default()
}

/// Construit le menu du clic droit.
///
/// La liste des sorties est figée au moment de la construction, d'où les
/// reconstructions déclenchées ailleurs : au branchement supposé, après un
/// basculement, et à l'ouverture du flyout.
fn build_tray_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let devices = audio_outputs();

    let mut menu = MenuBuilder::new(app);
    if devices.is_empty() {
        menu = menu.item(
            &MenuItemBuilder::with_id("audio:none", "Aucune sortie")
                .enabled(false)
                .build(app)?,
        );
    } else {
        for device in &devices {
            let item =
                CheckMenuItemBuilder::with_id(format!("{AUDIO_ITEM}{}", device.id), &device.name)
                    .checked(device.is_default)
                    .build(app)?;
            menu = menu.item(&item);
        }
    }

    menu.separator()
        .item(&MenuItemBuilder::with_id("open", "Ouvrir").build(app)?)
        .item(
            &MenuItemBuilder::with_id("sound-settings", "Réglages son")
                .build(app)?,
        )
        .separator()
        .item(&MenuItemBuilder::with_id("quit", "Quitter").build(app)?)
        .build()
}

/// Horodatage de la dernière reconstruction du menu.
#[derive(Default)]
struct MenuRefresh(Mutex<Option<Instant>>);

/// Intervalle minimal entre deux reconstructions non forcées.
const MENU_REBUILD_COOLDOWN: Duration = Duration::from_millis(500);

/// Reconstruit le menu du tray.
///
/// `force` contourne l'anti-rebond. À réserver aux cas où le contenu a
/// forcément changé (basculement de sortie), jamais aux événements de
/// souris : appeler `set_menu` pendant que Windows affiche le menu le fait
/// disparaître à l'écran.
fn refresh_tray_menu(app: &tauri::AppHandle, force: bool) {
    if !force {
        match app.try_state::<MenuRefresh>() {
            Some(state) => match state.0.lock() {
                Ok(mut last) => {
                    let now = Instant::now();
                    if last
                        .map(|prev| now.duration_since(prev) < MENU_REBUILD_COOLDOWN)
                        .unwrap_or(false)
                    {
                        return;
                    }
                    *last = Some(now);
                }
                Err(_) => return,
            },
            None => return,
        }
    }

    if let Some(tray) = app.tray_by_id(TRAY) {
        if let Ok(menu) = build_tray_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn on_menu(app: &tauri::AppHandle, id: &str) {
    match id {
        "quit" => app.exit(0),
        "open" => {
            if let Some(win) = app.get_webview_window(FLYOUT) {
                let anchor = app.state::<TrayAnchor>();
                position_flyout(&win, &anchor);
                let _ = win.show();
                let _ = win.set_focus();
            }
        }
        "sound-settings" => {
            let _ = std::process::Command::new("control.exe")
                .arg("mmsys.cpl")
                .spawn();
        }
        other => {
            let Some(device_id) = other.strip_prefix(AUDIO_ITEM) else {
                return;
            };
            if device_id == "none" {
                return;
            }
            let Ok(_com) = audio::ComGuard::new() else {
                return;
            };
            if audio::set_default_device(device_id).is_ok() {
                refresh_tray_menu(app, true);
            }
        }
    }
}

// --- Entrée -------------------------------------------------------------

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_monitors,
            set_brightness,
            reload_monitors,
            list_audio,
            set_audio,
            list_sessions,
            set_session_volume,
            set_session_mute,
            get_master,
            set_master_volume,
            set_master_mute,
            get_icon,
            set_flyout_height,
            get_autostart,
            set_autostart,
            save_capture,
            get_capture_preview,
            get_hovered_window,
            save_annotated_capture,
            copy_annotated_capture,
            close_capture,
            hide_toast,
            get_hotkey_error,
            get_hotkey_label,
            open_screenshots_dir,
            quit
        ])
        .setup(|app| {
            app.manage(DdcWorker::spawn());
            app.manage(TrayAnchor::default());
            app.manage(MenuRefresh::default());
            app.manage(Pending::default());
            app.manage(HotkeyError::default());

            // La fenêtre est créée une fois et gardée en vie. La détruire
            // économiserait de la RAM mais coûterait 200 à 500 ms de
            // recréation du webview à chaque ouverture.
            let win = WebviewWindowBuilder::new(app, FLYOUT, WebviewUrl::App("index.html".into()))
                .title("Toolbelt")
                .inner_size(FLYOUT_W, FLYOUT_H)
                .decorations(false)
                .resizable(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .transparent(true)
                .shadow(true)
                .visible(false)
                .build()?;

            // Se refermer dès qu'on clique ailleurs, comme les flyouts système.
            let w = win.clone();
            win.on_window_event(move |event| {
                if let WindowEvent::Focused(false) = event {
                    let _ = w.hide();
                }
            });

            // Overlay de sélection. Gardé en vie comme le flyout : le
            // raccourci doit afficher quelque chose tout de suite, pas
            // attendre 200 à 500 ms de création de webview. Sa taille est
            // fixée à chaque ouverture, sur les dimensions du bureau
            // virtuel du moment — celles-ci changent au branchement d'un
            // écran.
            let overlay =
                WebviewWindowBuilder::new(app, OVERLAY, WebviewUrl::App("overlay.html".into()))
                    .title("Capture")
                    .inner_size(100.0, 100.0)
                    .decorations(false)
                    .resizable(false)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .transparent(true)
                    .shadow(false)
                    .visible(false)
                    .build()?;

            // Le HWND vient du crate `windows` de Tauri (0.61), pas du nôtre
            // (0.58). On n'en garde que le pointeur brut : les deux versions
            // représentent le handle différemment, mais le pointeur, lui, ne
            // change pas.
            if let Ok(hwnd) = overlay.hwnd() {
                let hr = screenshot::disable_open_animation(hwnd.0 as *mut std::ffi::c_void);
                if hr != 0 {
                    eprintln!("DwmSetWindowAttribute a échoué (HRESULT {hr:#x})");
                }
            }

            // Confirmation d'enregistrement. Elle s'affiche pendant que
            // l'utilisateur est reparti travailler : elle ne doit pas
            // prendre le focus.
            //
            // Elle porte en revanche un bouton, donc elle ne peut plus être
            // rendue traversante (`set_ignore_cursor_events`) : ça
            // désensibiliserait le bouton avec le reste.
            //
            // Le prix est réel, et mesuré : tant qu'elle est affichée, c'est
            // **toute** sa surface qui absorbe les clics, bordure
            // transparente de l'ombre comprise. La fenêtre enfant de
            // WebView2 couvre la zone cliente entière et `WindowFromPoint`
            // la désigne partout, sans égard pour l'opacité peinte — une
            // fenêtre en couches n'est pas testée au pixel ici. Soit
            // 440 × 120 bloqués deux secondes et demie, dans le coin
            // bas-droit au-dessus de la barre des tâches.
            let toast = WebviewWindowBuilder::new(app, TOAST, WebviewUrl::App("toast.html".into()))
                .title("Capture enregistrée")
                .inner_size(TOAST_W, TOAST_H)
                .decorations(false)
                .resizable(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .transparent(true)
                .shadow(false)
                .focused(false)
                .visible(false)
                .build()?;

            if let Ok(hwnd) = toast.hwnd() {
                screenshot::never_activate(hwnd.0 as *mut std::ffi::c_void);
            }

            let hotkey_handle = app.handle().clone();
            if let Err(e) = hotkey::spawn(move || begin_capture(&hotkey_handle)) {
                // Un raccourci muet est indiscernable d'une application
                // plantée. On garde l'erreur pour l'afficher dans le flyout.
                eprintln!("raccourci : {e}");
                if let Ok(mut guard) = app.state::<HotkeyError>().0.lock() {
                    *guard = Some(e);
                }
            }

            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or("icône par défaut absente")?;

            let handle = app.handle().clone();

            TrayIconBuilder::with_id(TRAY)
                .icon(icon)
                .tooltip("Toolbelt")
                .menu(&build_tray_menu(&handle)?)
                // Sinon le clic gauche ouvrirait le menu au lieu du flyout.
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
                .on_tray_icon_event(|tray, event| {
                    let handle = tray.app_handle();
                    match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            rect,
                            ..
                        } => {
                            if let Some(win) = handle.get_webview_window(FLYOUT) {
                                toggle_flyout(&win, &handle.state::<TrayAnchor>(), rect);
                            }
                        }
                        // Le survol de l'icône précède forcément le clic
                        // droit, et se termine avant que le menu ne soit
                        // affiché. C'est le seul moment sûr pour reconstruire.
                        TrayIconEvent::Enter { .. } => refresh_tray_menu(handle, false),
                        _ => {}
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("échec du démarrage de Tauri");
}
