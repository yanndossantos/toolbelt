//! Outil de diagnostic. Objectif unique : savoir ce que le matériel de la
//! machine accepte réellement, et à quelle vitesse, avant d'écrire l'UI.
//!
//! Usage :
//!   probe                      diagnostic complet (à lancer en premier)
//!   probe monitors             liste les écrans + luminosité
//!   probe caps <i>             chaîne de capacités MCCS de l'écran i
//!   probe set <i> <0-100>      règle la luminosité de l'écran i
//!   probe bench <i>            mesure la latence DDC de l'écran i
//!   probe audio                liste les sorties audio
//!   probe audio-set <i>        bascule la sortie par défaut
//!   probe capture [chemin]     capture le bureau virtuel dans un PNG

use std::time::{Duration, Instant};

use toolbelt_audio as audio;
use toolbelt_capture as capture;
use toolbelt_ddc as ddc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("diag");

    let code = match cmd {
        "diag" => cmd_diag(),
        "monitors" => cmd_monitors(),
        "caps" => cmd_caps(arg_usize(&args, 1)),
        "set" => cmd_set(arg_usize(&args, 1), arg_u32(&args, 2)),
        "bench" => cmd_bench(arg_usize(&args, 1)),
        "audio" => cmd_audio(),
        "audio-set" => cmd_audio_set(arg_usize(&args, 1)),
        "capture" => cmd_capture(args.get(1).map(String::as_str)),
        other => {
            eprintln!("commande inconnue : {other}");
            usage();
            2
        }
    };

    std::process::exit(code);
}

fn usage() {
    eprintln!(
        "\nprobe [diag|monitors|caps <i>|set <i> <0-100>|bench <i>|audio|audio-set <i>|capture [chemin]]\n"
    );
}

// --- Capture ------------------------------------------------------------

/// Vérifie la chaîne BitBlt → GetDIBits → PNG de bout en bout, et chronomètre
/// la capture : c'est elle qui décide si l'overlay peut s'afficher sans que
/// l'on voie un temps mort après le raccourci.
fn cmd_capture(path: Option<&str>) -> i32 {
    let path = path.unwrap_or("capture.png");

    let start = Instant::now();
    let shot = match capture::capture_virtual_screen() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("capture : {e}");
            return 1;
        }
    };
    let grabbed = start.elapsed();

    println!(
        "bureau virtuel {}x{} à ({}, {}) — {}",
        shot.width,
        shot.height,
        shot.origin_x,
        shot.origin_y,
        ms(grabbed)
    );

    let start = Instant::now();
    if let Err(e) = shot.write_png(std::path::Path::new(path)) {
        eprintln!("écriture : {e}");
        return 1;
    }
    let written = start.elapsed();

    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    println!("{path} — {} Ko en {}", size / 1024, ms(written));
    0
}

fn arg_usize(args: &[String], i: usize) -> Option<usize> {
    args.get(i).and_then(|s| s.parse().ok())
}

fn arg_u32(args: &[String], i: usize) -> Option<u32> {
    args.get(i).and_then(|s| s.parse().ok())
}

fn ms(d: Duration) -> String {
    format!("{:.0} ms", d.as_secs_f64() * 1000.0)
}

// --- Écrans -------------------------------------------------------------

fn monitors() -> Vec<ddc::Monitor> {
    match ddc::enumerate() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("échec de l'énumération : {e}");
            Vec::new()
        }
    }
}

fn cmd_monitors() -> i32 {
    let list = monitors();
    if list.is_empty() {
        println!("Aucun écran DDC/CI détecté.");
        println!("Vérifier que DDC/CI est activé dans l'OSD de chaque écran.");
        return 1;
    }

    for (i, m) in list.iter().enumerate() {
        println!("\n[{i}] {} — {}", m.adapter, m.description);

        let t = Instant::now();
        match m.brightness() {
            Ok(v) => println!(
                "     luminosité : {}/{} ({} %)   lu en {}",
                v.current,
                v.max,
                v.percent(),
                ms(t.elapsed())
            ),
            Err(e) => println!("     luminosité : ÉCHEC — {e}   ({})", ms(t.elapsed())),
        }

        for (label, code) in [
            ("contraste     ", ddc::VCP_CONTRAST),
            ("source entrée ", ddc::VCP_INPUT_SOURCE),
            ("volume écran  ", ddc::VCP_AUDIO_VOLUME),
        ] {
            let t = Instant::now();
            match m.get_vcp(code) {
                Ok(v) => println!(
                    "     {label}: {}/{}   ({})",
                    v.current,
                    v.max,
                    ms(t.elapsed())
                ),
                Err(_) => println!("     {label}: non supporté   ({})", ms(t.elapsed())),
            }
        }
    }
    println!();
    0
}

fn cmd_caps(idx: Option<usize>) -> i32 {
    let Some(idx) = idx else {
        eprintln!("il manque l'index de l'écran");
        return 2;
    };
    let list = monitors();
    let Some(m) = list.get(idx) else {
        eprintln!("index hors limites (0..{})", list.len());
        return 2;
    };

    let t = Instant::now();
    match m.capabilities() {
        Ok(s) => {
            println!("lu en {}\n", ms(t.elapsed()));
            println!("{s}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_set(idx: Option<usize>, percent: Option<u32>) -> i32 {
    let (Some(idx), Some(percent)) = (idx, percent) else {
        eprintln!("usage : probe set <index> <0-100>");
        return 2;
    };
    let list = monitors();
    let Some(m) = list.get(idx) else {
        eprintln!("index hors limites (0..{})", list.len());
        return 2;
    };

    let t = Instant::now();
    match m.set_brightness_percent(percent) {
        Ok(()) => {
            println!("luminosité réglée à {percent} % en {}", ms(t.elapsed()));
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// C'est la mesure qui décide de la stratégie de l'UI : si un aller-retour
/// coûte 30 ms on peut suivre le slider presque en direct, si c'est 400 ms
/// il faut n'envoyer que la valeur finale.
fn cmd_bench(idx: Option<usize>) -> i32 {
    let Some(idx) = idx else {
        eprintln!("il manque l'index de l'écran");
        return 2;
    };
    let list = monitors();
    let Some(m) = list.get(idx) else {
        eprintln!("index hors limites (0..{})", list.len());
        return 2;
    };

    let initial = match m.brightness() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("impossible de lire la luminosité de départ : {e}");
            return 1;
        }
    };
    println!(
        "écran [{idx}] {} — départ {}/{}\n",
        m.description, initial.current, initial.max
    );

    let mut writes = Vec::new();
    let mut reads = Vec::new();
    let mut write_fail = 0;
    let mut read_fail = 0;

    // Valeurs proches pour éviter un clignotement désagréable.
    let a = initial.current.saturating_sub(initial.max / 10).max(1);
    let b = initial.current;

    for i in 0..10 {
        let target = if i % 2 == 0 { a } else { b };

        let t = Instant::now();
        let r = m.set_vcp(ddc::VCP_BRIGHTNESS, target);
        let d = t.elapsed();
        if r.is_ok() {
            writes.push(d);
        } else {
            write_fail += 1;
        }

        let t = Instant::now();
        let r = m.get_vcp(ddc::VCP_BRIGHTNESS);
        let d = t.elapsed();
        if r.is_ok() {
            reads.push(d);
        } else {
            read_fail += 1;
        }
    }

    let _ = m.set_vcp_retry(
        ddc::VCP_BRIGHTNESS,
        initial.current,
        3,
        Duration::from_millis(40),
    );

    report("écriture", &writes, write_fail);
    report("lecture ", &reads, read_fail);
    println!();
    0
}

fn report(label: &str, samples: &[Duration], failures: u32) {
    if samples.is_empty() {
        println!("{label} : 10 échecs sur 10");
        return;
    }
    let min = samples.iter().min().unwrap();
    let max = samples.iter().max().unwrap();
    let avg = samples.iter().sum::<Duration>() / samples.len() as u32;
    println!(
        "{label} : min {}, moy {}, max {}   ({} échecs sur 10)",
        ms(*min),
        ms(avg),
        ms(*max),
        failures
    );
}

// --- Audio --------------------------------------------------------------

fn cmd_audio() -> i32 {
    let _com = match audio::ComGuard::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("CoInitializeEx a échoué : {e}");
            return 1;
        }
    };

    let t = Instant::now();
    let devices = match audio::list_render_devices() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("énumération audio impossible : {e}");
            return 1;
        }
    };
    println!(
        "{} sortie(s) listée(s) en {}\n",
        devices.len(),
        ms(t.elapsed())
    );

    for (i, d) in devices.iter().enumerate() {
        let mark = if d.is_default { "*" } else { " " };
        println!("{mark}[{i}] {}", d.name);
        println!("      {}", d.id);
    }
    println!("\n* = sortie par défaut actuelle");
    0
}

fn cmd_audio_set(idx: Option<usize>) -> i32 {
    let Some(idx) = idx else {
        eprintln!("il manque l'index du périphérique");
        return 2;
    };

    let _com = match audio::ComGuard::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("CoInitializeEx a échoué : {e}");
            return 1;
        }
    };

    let devices = match audio::list_render_devices() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("énumération audio impossible : {e}");
            return 1;
        }
    };
    let Some(dev) = devices.get(idx) else {
        eprintln!("index hors limites (0..{})", devices.len());
        return 2;
    };

    let t = Instant::now();
    match audio::set_default_device(&dev.id) {
        Ok(()) => {
            println!("sortie par défaut : {} (en {})", dev.name, ms(t.elapsed()));
            0
        }
        Err(e) => {
            eprintln!("IPolicyConfig a refusé : {e}");
            eprintln!("(c'est ici que ça casserait si Microsoft changeait l'interface)");
            1
        }
    }
}

// --- Diagnostic complet -------------------------------------------------

fn cmd_diag() -> i32 {
    println!("=== ÉCRANS (DDC/CI) ===");
    let rc1 = cmd_monitors();

    println!("=== AUDIO (Core Audio) ===\n");
    let rc2 = cmd_audio();

    println!("\n=== SUITE ===");
    println!("- si des écrans sont listés : `probe bench 0` puis `probe caps 0`");
    println!("- pour tester le basculement audio : `probe audio-set <n>`");

    if rc1 != 0 || rc2 != 0 {
        1
    } else {
        0
    }
}
