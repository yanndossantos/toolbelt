//! Thread dédié au DDC/CI.
//!
//! Toute transaction DDC coûte ~60 ms sur cette machine et bloque. Elle ne
//! peut donc pas se faire sur le thread de l'UI. Ce worker possède les
//! handles écran et sérialise les accès.
//!
//! Le point important est la **fusion des commandes** : quand on tire un
//! slider, l'UI émet une valeur par image. À 60 ms par écriture, on ne peut
//! en absorber qu'environ 16 par seconde. Le worker vide donc sa file à
//! chaque tour et ne garde que la dernière valeur demandée par écran — les
//! valeurs intermédiaires n'ont aucun intérêt, seule la position finale du
//! doigt compte.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use toolbelt_ddc as ddc;

/// Ce que l'UI voit d'un écran.
#[derive(Debug, Clone, Serialize)]
pub struct MonitorInfo {
    pub index: usize,
    pub name: String,
    pub brightness: u32,
    /// Faux si le dernier échange DDC a échoué.
    pub available: bool,
}

pub enum Cmd {
    Reload,
    SetBrightness { index: usize, percent: u32 },
}

/// Handle vers le worker. Clonable, utilisable depuis n'importe quel thread.
pub struct DdcWorker {
    tx: Sender<Cmd>,
    state: Arc<Mutex<Vec<MonitorInfo>>>,
}

impl DdcWorker {
    pub fn spawn() -> Self {
        let (tx, rx) = channel();
        let state = Arc::new(Mutex::new(Vec::new()));
        let state_thread = Arc::clone(&state);

        thread::spawn(move || run(rx, state_thread));

        DdcWorker { tx, state }
    }

    /// Dernier état connu. Ne touche pas au matériel, retour immédiat.
    pub fn snapshot(&self) -> Vec<MonitorInfo> {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Redemande une énumération complète. À appeler au réveil de veille
    /// et au hotplug écran, où les handles deviennent invalides.
    pub fn reload(&self) {
        let _ = self.tx.send(Cmd::Reload);
    }

    /// Demande un réglage. Non bloquant : la valeur peut être écrasée par
    /// une demande plus récente avant d'atteindre l'écran.
    pub fn set_brightness(&self, index: usize, percent: u32) {
        let _ = self.tx.send(Cmd::SetBrightness { index, percent });
        // Optimisme : on met l'état à jour tout de suite pour que l'UI
        // reste fluide. Le worker corrigera si l'écran refuse.
        if let Ok(mut s) = self.state.lock() {
            if let Some(m) = s.iter_mut().find(|m| m.index == index) {
                m.brightness = percent.min(100);
            }
        }
    }
}

/// Un écran + son maximum VCP, mis en cache.
///
/// Sans ce cache il faudrait une lecture (60 ms) avant chaque écriture
/// (60 ms) pour connaître l'échelle. Le max ne change jamais en cours de vie
/// d'un handle, donc on le lit une seule fois à l'énumération.
struct Slot {
    mon: ddc::Monitor,
    max: u32,
    /// Dernière valeur connue en pourcent. Mise à jour à l'énumération puis
    /// à chaque écriture réussie. On ne relit jamais l'écran pour publier
    /// l'état : ça coûterait 60 ms de plus par écran et par cycle.
    percent: u32,
    available: bool,
}

fn run(rx: Receiver<Cmd>, state: Arc<Mutex<Vec<MonitorInfo>>>) {
    let mut slots: Vec<Slot> = Vec::new();
    reload(&mut slots, &state);

    loop {
        // Bloque jusqu'à la première commande, puis vide la file.
        let first = match rx.recv() {
            Ok(c) => c,
            Err(_) => break, // tous les émetteurs sont tombés : on sort
        };

        let mut want_reload = false;
        let mut pending: HashMap<usize, u32> = HashMap::new();

        for cmd in std::iter::once(first).chain(rx.try_iter()) {
            match cmd {
                // Une réénumération renumérote les écrans : un réglage demandé
                // *avant* elle ne désigne plus forcément le même, on l'oublie.
                // Ceux qui arrivent après, en revanche, sont valides et doivent
                // survivre — sinon un lot contenant un Reload avalerait
                // silencieusement le geste en cours sur un slider.
                Cmd::Reload => {
                    want_reload = true;
                    pending.clear();
                }
                // insert écrase : dernière valeur gagne.
                Cmd::SetBrightness { index, percent } => {
                    pending.insert(index, percent.min(100));
                }
            }
        }

        if want_reload {
            reload(&mut slots, &state);
        }

        for (index, percent) in pending {
            if let Some(slot) = slots.get_mut(index) {
                let max = slot.max.max(1);
                let raw = ((percent as u64 * max as u64) / 100) as u32;
                let ok = slot
                    .mon
                    .set_vcp_retry(ddc::VCP_BRIGHTNESS, raw, 2, Duration::from_millis(40))
                    .is_ok();
                slot.available = ok;
                if ok {
                    slot.percent = percent;
                }
            }
        }

        publish(&slots, &state);
    }
}

fn reload(slots: &mut Vec<Slot>, state: &Arc<Mutex<Vec<MonitorInfo>>>) {
    // Vider d'abord : le Drop libère les anciens handles, qui peuvent être
    // invalides si on arrive ici après une veille.
    slots.clear();

    let monitors = match ddc::enumerate() {
        Ok(m) => m,
        Err(_) => Vec::new(),
    };

    for mon in monitors {
        // Seul endroit où on lit vraiment l'écran.
        let (max, percent, available) = match mon.brightness() {
            Ok(v) => (v.max.max(1), v.percent(), true),
            Err(_) => (100, 0, false),
        };
        slots.push(Slot {
            mon,
            max,
            percent,
            available,
        });
    }

    publish(slots, state);
}

fn publish(slots: &[Slot], state: &Arc<Mutex<Vec<MonitorInfo>>>) {
    let infos: Vec<MonitorInfo> = slots
        .iter()
        .enumerate()
        .map(|(index, slot)| MonitorInfo {
            index,
            name: display_name(index, slot),
            brightness: slot.percent,
            available: slot.available,
        })
        .collect();

    if let Ok(mut s) = state.lock() {
        *s = infos;
    }
}

/// TODO : remplacer par QueryDisplayConfig + DISPLAYCONFIG_TARGET_DEVICE_NAME.
/// Le pilote renvoie « Generic PnP Monitor » pour tout le monde, ce qui ne
/// permet pas de distinguer deux écrans dans l'UI.
fn display_name(index: usize, slot: &Slot) -> String {
    let desc = slot.mon.description.trim();
    if desc.is_empty() || desc.eq_ignore_ascii_case("Generic PnP Monitor") {
        format!("Écran {}", index + 1)
    } else {
        desc.to_string()
    }
}
