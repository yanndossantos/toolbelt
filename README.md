# Toolbelt

Boîte à outils Windows dans la zone de notification. Quatre fonctions :
luminosité des écrans en DDC/CI, choix de la sortie audio, mélangeur de
volume par application, et capture d'écran par sélection.

```
toolbelt/
├── crates/ddc/      DDC/CI — aucune dépendance, FFI Win32 écrite à la main
├── crates/audio/    Core Audio, IPolicyConfig, sessions, icônes
├── crates/capture/  BitBlt + GetDIBits, recadrage, encodage PNG
├── probe/           CLI de diagnostic matériel
├── app/             Tauri 2 — tray + flyout + overlay de capture
└── install.ps1      compile en release et installe
```

## Prérequis

- `rustup` avec la toolchain `stable-x86_64-pc-windows-msvc`
- Build Tools for Visual Studio, composant « Desktop development with C++ »
  (pour `link.exe` et les `.lib` système)
- WebView2, déjà présent sur Windows 11

## Installer

```powershell
cd C:\chemin\vers\toolbelt
.\install.ps1
```

Le script compile en release, arrête l'instance en cours, copie l'exe dans
`%LOCALAPPDATA%\Toolbelt` et le relance.

Si l'exécution de scripts est bloquée :

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

### À la main

```powershell
cargo build --release
mkdir "$env:LOCALAPPDATA\Toolbelt" -Force
copy .\target\release\toolbelt.exe "$env:LOCALAPPDATA\Toolbelt\"
& "$env:LOCALAPPDATA\Toolbelt\toolbelt.exe"
```

Le binaire est dans `target\release\toolbelt.exe`. Il est **autonome** :
Tauri embarque `index.html` et les icônes dedans, il n'y a pas de dossier
`ui` à trimballer. Un seul fichier à déplacer ou à sauvegarder.

La copie dans `%LOCALAPPDATA%` n'est pas décorative : le démarrage
automatique enregistre le chemin de l'exe au moment où on coche la case. Si
ce chemin est dans `target\`, un `cargo clean` casse le démarrage en silence.

### Mettre à jour

Recompiler, puis recopier l'exe par-dessus l'ancien. Le chemin ne bouge pas,
donc rien à recocher. `install.ps1` s'en charge, y compris d'arrêter
l'instance en cours qui verrouille le fichier.

## Développer

```powershell
cargo run -p toolbelt-app
```

Attention, en debug la console reste ouverte : `windows_subsystem =
"windows"` n'est actif qu'en release. C'est normal, et c'est la raison pour
laquelle l'usage quotidien doit passer par la version release.

La première compilation est longue, Tauri tire beaucoup de dépendances. Il
n'y a pas de npm : le frontend est un `index.html` statique servi tel quel,
sans bundler.

## Utiliser

Une icône apparaît dans la zone de notification.

- **Clic gauche** : ouvre le flyout. Clic ailleurs pour le refermer.
- **Clic droit** : menu avec les sorties audio (la sortie active est cochée),
  « Ouvrir » et « Quitter ».

Le flyout a aussi un bouton « Quitter » dans son pied.

### Capture d'écran

**Win+Shift+X** assombrit l'écran ; on trace un rectangle à la souris. Le
tracé relâché, le rectangle reste ajustable : huit poignées pour le
redimensionner, un glisser à l'intérieur pour le déplacer, un tracé ailleurs
pour repartir de zéro.

Deux boutons apparaissent alors, collés au rectangle et non aux coins de
l'écran — l'overlay fait la largeur du bureau virtuel, un coin d'écran peut
être à des milliers de pixels de la sélection :

- **Enregistrer**, en bas à gauche, écrit le PNG dans
  `%USERPROFILE%\Pictures\Toolbelt Screenshots` sous le nom
  `Capture AAAA-MM-JJ HHMMSS.png`, et referme l'outil aussitôt. La
  confirmation s'affiche en bas à droite de l'écran principal et s'efface
  d'elle-même après 2,6 s. Elle porte un bouton **Ouvrir** qui ouvre le
  dossier des captures dans l'Explorateur ; la survoler suspend le compte à
  rebours, sinon le bouton serait souvent hors d'atteinte.
- Le bouton **Enregistrer et ouvrir dans une fenêtre** de la barre d'outils
  enregistre le même PNG, puis l'affiche directement dans une fenêtre séparée.
  Cette fenêtre reste disponible pour consulter la capture sans ouvrir le
  fichier depuis l'Explorateur.
- **✕**, en haut à droite, ferme sans rien écrire.

Après la sélection, la barre d'outils permet d'ajouter un rectangle, un
cercle, une flèche, du texte ou un dessin libre. La couleur, l'épaisseur du
trait et la taille du texte sont réglables. Les boutons d'annulation,
rétablissement et suppression permettent de corriger les annotations; les
raccourcis `Ctrl+Z`, `Ctrl+Y` et `Suppr` sont également disponibles. Les
annotations sont intégrées au PNG enregistré et les contrôles de l'overlay ne
font pas partie de l'image.

La poignée en forme de points à gauche de la barre permet de la déplacer
librement sans déplacer la sélection.

Échap ferme aussi, et un clic sans glisser vaut renoncement.

Les deux boutons basculent de l'autre côté du rectangle quand ils
sortiraient de l'écran : une sélection collée au bord haut garde sa croix.

Le pied du flyout rappelle le raccourci et ouvre ce dossier. Le libellé vient
de `get_hotkey_label`, pas d'une chaîne écrite en dur dans la page : il ne
peut pas mentir sur ce qui est réellement enregistré.

Le PNG est sans perte : les pixels relus sont exactement ceux de l'écran. Le
fichier est compressé, mais la compression PNG est réversible — rien n'est
jeté. Une vraie absence de compression voudrait dire du BMP, 33 Mo pour un
plein écran 4K contre 3,5 Mo ici.

La sélection porte sur tous les écrans à la fois : l'overlay couvre le
bureau virtuel entier, on peut donc tracer un rectangle à cheval sur deux
écrans.

Si le raccourci était déjà pris par une autre application, l'app le signale
dans ce même pied au lieu de rester muette.

Si l'icône n'apparaît pas, regarder dans le débordement de la zone de
notification (la petite flèche). Windows y range les nouvelles icônes par
défaut ; on la remonte en la glissant sur la barre des tâches.

Pour vérifier le démarrage automatique :

```powershell
Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" |
  Select-Object Toolbelt
```

## Diagnostic matériel

Le CLI reste utile quand quelque chose ne répond plus.

```powershell
cargo run -p toolbelt-probe                    # état complet
cargo run -p toolbelt-probe -- monitors        # écrans + luminosité
cargo run -p toolbelt-probe -- bench 0         # latence DDC de l'écran 0
cargo run -p toolbelt-probe -- caps 0          # capacités MCCS brutes
cargo run -p toolbelt-probe -- set 0 50        # luminosité à 50 %
cargo run -p toolbelt-probe -- audio           # sorties audio
cargo run -p toolbelt-probe -- audio-set 1     # bascule la sortie
cargo run -p toolbelt-probe -- capture out.png # capture + chrono
```

`capture` court-circuite le raccourci et l'overlay : il vérifie la seule
chaîne BitBlt → GetDIBits → PNG, et la chronomètre. Sur cette machine,
80 ms pour saisir 6000×1440 et 29 ms pour l'encoder.

## Mesures sur cette machine

Écrans en DisplayPort, RX 9070 XT :

| | min | moy | max | échecs |
|---|---|---|---|---|
| écriture VCP | 59 ms | 60 ms | 61 ms | 0 / 10 |
| lecture VCP | 63 ms | 63 ms | 64 ms | 0 / 10 |

Basculement audio via `IPolicyConfig` : 16 à 17 ms pour les trois rôles.

La variance de 2 ms indique un coût fixe imposé par le pilote, pas le temps
réel du bus I²C. C'est ce qui justifie le bridage du slider à 60 ms et la
fusion des commandes dans le worker.

## Choix de conception

**Worker DDC.** Une transaction coûte 60 ms et bloque. Un thread dédié
possède les handles, et fusionne les commandes en attente : quand on tire un
slider, seule la dernière valeur par écran est envoyée.

Une réénumération dans le même lot annule les réglages qui la précèdent —
elle renumérote les écrans, l'index visé ne désigne peut-être plus le même.
Ceux qui la suivent sont appliqués normalement. La distinction comptera le
jour où le reload deviendra automatique (point 1 de la liste plus bas) : tout
jeter ferait disparaître le geste en cours sur un slider.

**Icônes.** Extraites de l'exécutable via `ExtractIconExW` puis `GetDIBits`
(`crates/audio/src/icons.rs`), mises en cache par PID côté frontend. Sans ce
cache, le sondage à la seconde relancerait une extraction GDI complète pour
chaque application.

**Démarrage automatique.** Écriture directe de
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, valeur `Toolbelt`
(`app/src/autostart.rs`). Pas de plugin : le comportement est visible et les
erreurs remontent avec leur code jusque dans le flyout. La case n'est cochée
que si l'entrée pointe sur **cet** exécutable-ci ; une entrée laissée par une
ancienne copie est signalée séparément.

**Hauteur de la fenêtre.** Pas de barre de défilement : le frontend mesure
son contenu et appelle `set_flyout_height`, qui redimensionne et replace la
fenêtre en gardant son bord bas au-dessus de l'icône du tray.

**Menu du tray reconstruit au survol.** Un menu Tauri est figé une fois
construit. On le refait quand la souris entre sur l'icône — ce qui précède
forcément le clic droit et se termine avant l'affichage du menu — et après
un basculement de sortie. Aucun sondage.

Ne jamais appeler `set_menu` sur un événement de clic droit : Windows est
alors en train d'afficher le menu, et le remplacer le fait clignoter puis
s'effondrer. Un anti-rebond de 500 ms limite en plus les rafales
d'événements de survol.

**Flyout gardé en vie.** La fenêtre n'est jamais détruite. Ça coûte de la
RAM, mais la recréer prendrait 200 à 500 ms à chaque ouverture. L'overlay de
capture suit la même règle, pour la même raison.

**Capture d'abord, sélection ensuite.** L'appui sur le raccourci fige tout
le bureau virtuel, *puis* affiche l'overlay. Le recadrage se fait ensuite en
mémoire, sur ces pixels-là.

L'ordre inverse — sélectionner puis capturer — obligerait à masquer
l'overlay et à parier sur le fait que Windows ait fini de repeindre avant le
`BitBlt`. Ici la question ne se pose pas : l'overlay ne peut pas se
photographier lui-même, et son assombrissement n'apparaît jamais dans le
fichier. Ça se vérifie à l'œil sur n'importe quelle capture.

Le coût est un buffer de la taille du bureau virtuel — 25 Mo en RVB pour
6000×1440 — gardé le temps de la sélection, et libéré dès la validation ou
l'annulation.

**Raccourci sans fenêtre.** `RegisterHotKey(NULL, ...)` rattache le
raccourci au thread appelant, qui reçoit `WM_HOTKEY` dans sa file. D'où le
thread dédié avec sa boucle `GetMessageW` : elle bloque, elle ne peut pas
cohabiter avec celle de l'UI.

`Win+Shift+X` a été choisi parce qu'il est libre : `Win+X` (menu système) et
`Win+Shift+S` (outil Capture de Windows) refusent tous deux
l'enregistrement avec le code 1409.

**Animation d'ouverture coupée.** Windows déploie une fenêtre depuis son
centre à l'affichage. Sur l'overlay, qui couvre le bureau virtuel entier, ce
centre tombe à la jointure entre les écrans : l'assombrissement arrive de
travers sur chacun. `DwmSetWindowAttribute` avec
`DWMWA_TRANSITIONS_FORCEDISABLED` supprime l'effet, et `overlay.html` le
remplace par un fondu CSS de 110 ms qui couvre toute la surface d'un coup.

Le `HWND` vient du crate `windows` de Tauri (0.61), pas du nôtre (0.58). On
n'en garde que le pointeur brut : les deux versions représentent le handle
différemment, mais le pointeur ne change pas.

**L'enregistrement emprunte la capture, il ne la consomme pas.**
`save_capture` lit l'image figée sans la retirer ; seul `close_capture` la
libère. C'est ce qui rend le mode ajustement possible — on peut réenregistrer
après avoir bougé le rectangle — et surtout ce qui laisse de quoi réessayer
quand l'écriture échoue. Consommer le buffer rendrait l'échec définitif :
l'outil resterait ouvert en affichant une erreur devant un rectangle devenu
inutilisable.

**La confirmation est une fenêtre à part.** L'overlay couvre le bureau
virtuel entier : y afficher le chemin obligerait à le laisser en place le
temps de le lire, donc à bloquer la machine pour une information. La
confirmation vit dans sa propre petite fenêtre, et l'outil disparaît dès le
clic.

`WS_EX_NOACTIVATE` l'empêche de prendre le focus : `show()` de Tauri passe
par `ShowWindow(SW_SHOW)`, qui **active** la fenêtre — sans ce style, la
confirmation volerait le clavier à ce qu'on est en train de taper.

Elle a porté un temps `set_ignore_cursor_events`, qui laissait passer tous
les clics. Le bouton « Ouvrir » l'a rendu intenable : il désensibilise la
fenêtre entière, bouton compris. Elle est donc redevenue sensible, et tant
qu'elle est affichée sa surface absorbe les clics — y compris la bordure
transparente réservée à l'ombre. On pourrait croire qu'une fenêtre en
couches est testée au pixel selon son opacité ; elle ne l'est pas ici,
`WindowFromPoint` désigne la fenêtre enfant de WebView2 partout dans la zone
cliente. Cela fait 440 × 120 bloqués deux secondes et demie, dans un coin,
au-dessus de la barre des tâches.

Elle entre et sort en fondu. La sortie impose de retarder le masquage :
`hide_toast` fait disparaître la fenêtre instantanément, l'appeler dès la fin
du délai d'affichage escamoterait l'animation. Le minuteur pose donc la
classe `leaving`, et un second, calé sur la durée du `@keyframes`, masque la
fenêtre une fois le fondu terminé.

`TOAST_SHADOW` mérite un mot : la fenêtre est plus grande que la carte
visible, de cette bordure transparente sur chaque côté, parce qu'une fenêtre
au ras de la carte tranche l'ombre portée net — `overflow: hidden` ne
pardonne pas. Cette bordure est retranchée du positionnement, sinon la carte
flotterait loin du coin. Elle doit rester égale à la marge de `#card` dans
`toast.html`, et au moins aussi grande que le rayon de flou de l'ombre.

Son contenu est poussé par `eval`, pas par le système d'événements de Tauri.
`listen` n'y rappelle jamais — ni ciblé par label, ni en émission globale —
parce que ce projet ne déclare aucune capacité ACL : `invoke` de nos propres
commandes n'en a pas besoin, l'API d'événements si. Le symptôme est trompeur,
la fenêtre s'affiche mais vide, et comme c'est l'écouteur qui arme le
minuteur d'effacement, elle ne disparaît plus jamais. À garder en tête avant
de vouloir ajouter un événement ici.

Conséquence assumée : une confirmation encore affichée au moment où l'on
relance le raccourci se retrouve dans la capture. La masquer avant le
`BitBlt` supposerait d'attendre que Windows ait repeint, exactement le pari
que l'ordre « capture d'abord » évite. Ce qui est à l'écran est ce qui est
enregistré, y compris nous.

**Un seul assombrissement à la fois.** Tant qu'aucun rectangle n'est tracé,
c'est `#dim` qui assombrit. Dès le premier clic il disparaît et l'ombre
portée de `#sel` prend le relais, ce qui perce un trou net. Les garder tous
les deux doublerait l'opacité hors sélection et, surtout, assombrirait aussi
la zone sélectionnée — l'aperçu ne correspondrait plus au fichier écrit.

## Points fragiles connus

**Vtable d'IPolicyConfig.** Les dix méthodes bouchons déclarées avant
`SetDefaultEndpoint` ne sont pas décoratives : en COM une méthode est
identifiée par sa position. Ne rien ajouter ni retirer au-dessus d'elle.

**Handles DDC invalidés.** Ils deviennent silencieusement invalides au réveil
de veille, au hotplug écran et à la mise à jour du pilote GPU. `reload()`
existe sur le worker mais rien ne l'appelle encore automatiquement : il
faudra écouter `WM_DISPLAYCHANGE` et `WM_POWERBROADCAST`. C'est la cause des
« No compatible displays found » récurrents de Twinkle Tray.

**Un sondage subsiste.** Faute d'écouter `IAudioSessionNotification`, le
mélangeur réinterroge le système une fois par seconde — mais uniquement
quand le flyout est ouvert. Fermé, l'app ne fait rien.

**Noms d'écrans.** Le pilote renvoie « Generic PnP Monitor » pour tout le
monde, donc l'UI affiche « Écran 1 » et « Écran 2 ». Le vrai nom passe par
`QueryDisplayConfig` + `DISPLAYCONFIG_TARGET_DEVICE_NAME`.

**HDR.** Sur certaines dalles le VCP 0x10 est ignoré quand le HDR est actif.

**L'overlay n'est capturable par aucun outil de capture.** Ni `BitBlt` depuis
le DC écran, ni `PrintWindow` avec `PW_RENDERFULLCONTENT` ne rendent son
contenu : c'est une fenêtre transparente dont WebView2 compose le rendu hors
du chemin GDI. Pratique — il ne peut pas polluer nos propres captures — mais
ça veut dire qu'on ne peut pas vérifier son apparence en le photographiant.
Le contournement — le rendre hors de Tauri dans un Edge headless — est
décrit plus bas, dans l'état de la vérification.

**Sélection sur des écrans à mise à l'échelle différente.** L'overlay couvre
le bureau virtuel entier, mais une fenêtre Windows n'a qu'un seul facteur
d'échelle. `save_capture` convertit les pixels CSS du frontend en pixels
physiques avec ce facteur unique. Sur des écrans réglés à la même mise à
l'échelle — le cas ici — la conversion est exacte. Sur un mélange 100 % /
150 %, la sélection dérivera sur le second écran. Le correctif est de
convertir écran par écran à partir de `MONITORINFO`, pas de la fenêtre.

**Codes VCP non continus.** Le probe affiche le code 0x60 (source d'entrée)
comme `courant/max`, ce qui n'a pas de sens : c'est une énumération, pas une
grandeur. D'où le `15/3` incohérent.

## État de la vérification

Type-checkés hors Windows : `crates/ddc`, `probe`, `app/src/ddc_worker.rs`,
`app/src/autostart.rs`, `crates/audio/src/icons.rs` (dont l'encodeur base64,
testé contre les vecteurs de la RFC 4648).

La capture est la partie la mieux vérifiée. `crates/capture` sépare
volontairement le recadrage — pur, sans FFI — du `BitBlt`, ce qui le rend
testable partout : quatre tests couvrent le rendu des bons pixels, le
rognage sur les bords, la sélection entièrement hors champ, et l'origine
négative du bureau virtuel (le cas qui casse dès qu'un écran est placé à
gauche du principal, ce qui est la configuration de cette machine).

Le reste a été vérifié en pilotant l'application réelle : raccourci envoyé
par `keybd_event`, glisser simulé par `mouse_event`, puis contrôle que le
PNG produit fait exactement les dimensions demandées et contient la bonne
zone, sans l'assombrissement de l'overlay.

**Inspecter l'overlay sans l'application.** Puisqu'il est incapturable une
fois affiché (voir les points fragiles), on le rend hors de Tauri : copier
`overlay.html`, y injecter un `window.__TAURI__` bouchon, un fond d'écran
factice, et un script final qui pose un rectangle et appelle `setMode` et
`render`. Le moteur de WebView2 est celui d'Edge, dont le binaire est déjà
sur la machine :

```powershell
& "C:\Program Files (x86)\Microsoft\EdgeCore\<version>\msedge.exe" `
  --headless --disable-gpu --allow-file-access-from-files `
  --window-size=1280,760 --screenshot=out.png file:///.../test.html
```

Attention en préparant ce fichier : `Get-Content` de PowerShell 5.1 lit en
ANSI et corrompt les non-ASCII. Le `×` de l'étiquette de taille ressort en
`Ã—`, ce qui ressemble à un bug de l'application alors que le fichier source
est en UTF-8 valide.

Jamais compilés hors Windows, donc plus exposés : le reste de
`crates/audio`, et `app/src/main.rs` qui utilise une large surface de l'API
Tauri 2.

## Ensuite

1. Réénumération automatique sur changement d'écran et sortie de veille
2. Vrais noms d'écrans via `QueryDisplayConfig`
3. `IAudioSessionNotification` et `IMMNotificationClient`, pour supprimer le
   dernier sondage périodique
4. Icônes de meilleure qualité (`SHGetFileInfoW` donne l'icône du shell,
   souvent plus nette que celle de l'exécutable)
5. Redimensionner au clavier (flèches) et valider par Entrée — la souris est
   seule à piloter le mode ajustement
