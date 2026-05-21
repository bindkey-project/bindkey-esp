<p align="center">
  <img src="assets/logo-bindkey.png" alt="Logo BindKey" width="220"/>
</p>

<h1 align="center">BindKey</h1>
<h3 align="center">bindkey-esp — firmware slave</h3>

**BindKey** est un proxy USB chiffré qui se branche entre l'ordinateur et un
support de stockage externe (clé USB, SSD, lecteur SD). Toutes les données qui
transitent par BindKey sont scellées et chiffrées en AES-256-GCM par un
microcontrôleur sécurisé ; elles ne sont déchiffrées que pour les utilisateurs
légitimes, authentifiés biométriquement, dont la BindKey détient les droits
cryptographiques sur le volume concerné. Toute modification effectuée en dehors
de l'environnement BindKey rend le contenu illisible. La solution fonctionne
**hors cloud**, sans driver côté hôte, et permet en complément à un logiciel
desktop et à un serveur de gérer la délégation d'accès entre BindKeys d'une
même organisation pour un partage collaboratif sécurisé.

Ce repo contient le firmware **slave** (ESP32-S3 N°2) — l'une des deux MCU
qui composent BindKey.

---

# bindkey-esp

BindKey - Master Project - ESP32#2 Code Repository

> Firmware Rust de la BindKey **slave** — ESP32-S3 N°2 du projet BindKey.

`bindkey-esp` est l'un des deux firmwares qui composent le proxy USB chiffré
**BindKey**. Cette MCU se trouve « derrière » la première (`bindkey-tinyesp`,
voir repo dédié) : elle ne parle jamais directement au PC hôte. Son rôle est
double :

1. **Recevoir les commandes** du premier ESP32-S3 sur un bus SPI3 en
   esclave (lecture/écriture de secteurs, GetStatus, GetCapacity, Flush).
2. **Piloter le vrai média de stockage** USB (clé USB, SSD externe, lecteur
   carte SD-USB…) via une pile USB Host MSC.

Du point de vue du PC, ce firmware est invisible : il n'expose **aucun**
endpoint USB côté hôte. Tout passe par la première MCU.

---

## Place dans l'architecture BindKey

```
        Client PC
            │  USB (driver OS natif, classe MSC)
            ▼
  ┌─────────────────────────────┐
  │   bindkey-tinyesp  (master) │
  │   ESP32-S3 #1               │
  │   • Émulation USB MSC       │
  │   • Auth biométrique        │
  │   • Chiffrement AES-GCM     │
  └─────────────┬───────────────┘
                │  SPI3 inter-MCU
                ▼
  ┌─────────────────────────────┐
  │   bindkey-esp  (slave)      │   ← CE REPO
  │   ESP32-S3 #2               │
  │   • SPI slave               │
  │   • USB Host MSC            │
  │   • Gestion power switch    │
  └─────────────┬───────────────┘
                │  USB-OTG (host)
                ▼
        Clé USB physique
        (FAT/exFAT/NTFS/ext4)
```

L'isolation entre les deux MCU permet de garantir que la clé physique est
**inaccessible** sans passer par le chemin chiffré du master. Aucun secret
(clé volume, template biométrique, clé ECDH) n'existe sur cette carte.

---

## Fonctionnalités

- Pile **USB Host MSC** via `esp_usb` et un composant C custom
  (`src/usb_mass_storage/usb_host_msc/`) qui expose un block-device :
  `bd_status()`, `bd_read_blocks()`, `bd_write_blocks()`
- **SPI slave** sur SPI3, frame de 8212 octets max (16 B header + 8192 B
  payload + 4 B CRC32), commandes `GetStatus`, `GetCapacity`, `Read`, `Write`,
  `Flush`
- Gestion d'alimentation du port USB host via un switch **TPS** (GPIO 1 =
  `EN_BARRE`), avec power-cycle automatique en cas d'erreur USB persistante
- LED de diagnostic (GPIO 8)
- Robustesse hotplug : reconnexion à chaud d'une clé USB, retry x3 avec
  power-cycle, capacité rafraîchie au montage

---

## Matériel requis

| Élément           | Référence                                                      |
|-------------------|----------------------------------------------------------------|
| MCU               | ESP32-S3 (n'importe quelle dev-board avec USB Host accessible) |
| Switch USB Host   | Texas Instruments TPS (ou équivalent avec pin EN)              |
| Port USB-A        | exposé côté média physique                                     |
| Lien inter-MCU    | 5 fils SPI3 + ligne READY depuis la carte `bindkey-tinyesp`    |
| Média de stockage | clé USB / SSD externe / lecteur SD-USB compatible USB MSC      |

---

## Prérequis logiciels

Ce firmware utilise la toolchain **Rust pour Xtensa** maintenue par
Espressif, distincte de la toolchain Rust standard. Mêmes outils que pour
`bindkey-tinyesp`.

### 1. Rust + `rustup`

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. `espup` (installateur de la toolchain Xtensa + ESP-IDF)

```bash
cargo install espup --locked
espup install
. $HOME/export-esp.sh        # à sourcer dans chaque shell
```

`espup install` installe le compilateur `rustc` patché pour Xtensa, GCC
Xtensa, LLVM, et configure les variables d'environnement nécessaires à
`esp-idf-sys` (qui appellera la build ESP-IDF C en arrière-plan).

### 3. Outils de flash et de scaffolding

```bash
cargo install ldproxy           # linker proxy utilisé par esp-idf-sys
cargo install espflash --locked # flash + monitor série
cargo install cargo-generate    # pour créer de nouveaux projets ESP-IDF
```

### 4. (Optionnel) Création d'un projet ESP-IDF + Rust « from scratch »

Si tu veux régénérer un squelette équivalent à ce repo :

```bash
cargo generate esp-rs/esp-idf-template cargo
```

Réponses à donner au template :
- **MCU** : `esp32s3`
- **ESP-IDF version** : `v5.3.3` (cf. `.cargo/config.toml` de ce repo)
- **STD support** : `true` (utilisé ici)

### Dépendances système (Linux/macOS)

```bash
# Debian/Ubuntu
sudo apt install -y git wget flex bison gperf python3 python3-pip python3-venv \
                    cmake ninja-build ccache libffi-dev libssl-dev dfu-util \
                    libusb-1.0-0 pkg-config
```

---

## Versions verrouillées (extrait `Cargo.toml`)

| Élément          | Version                                       |
|------------------|-----------------------------------------------|
| Edition Rust     | 2021                                          |
| `rust-version`   | ≥ 1.77                                        |
| Toolchain Rust   | `channel = "esp"` (cf. `rust-toolchain.toml`) |
| Target           | `xtensa-esp32s3-espidf`                       |
| ESP-IDF          | `v5.3.3`                                      |
| `esp-idf-svc`    | `0.51`                                        |
| `esp-idf-sys`    | `0.36` (feature `native`)                     |
| `embuild`        | `0.33`                                        |

Composant C custom embarqué : `src/usb_mass_storage/usb_host_msc/` (binding
généré dans le module Rust `usb_msc` via `bindgen`).

---

## Build, flash, monitor

```bash
git clone <url> bindkey-esp
cd bindkey-esp

# Activer la toolchain ESP
. $HOME/export-esp.sh

# Build debug
cargo build

# Build release
cargo build --release

# Build + flash + monitor série (commande principale)
cargo run

# Lint
cargo clippy -- -D warnings
```

> ⚠️ Comme pour `bindkey-tinyesp` : **toujours** utiliser `cargo build` /
> `cargo run`. Ne pas invoquer `idf.py build` directement — `esp-idf-sys`
> orchestre l'intégralité de la build ESP-IDF.

---

## Configuration `sdkconfig.defaults`

Les éléments structurants pour ce slave :

```ini
CONFIG_ESP_MAIN_TASK_STACK_SIZE=24576
CONFIG_FREERTOS_HZ=1000

# Support FAT avec noms longs (utile pour les clés en exFAT/FAT32)
CONFIG_FATFS_LFN_HEAP=y
CONFIG_FATFS_MAX_LFN=255
```

La pile USB Host MSC est activée via le composant intégré
`src/usb_mass_storage/usb_host_msc/` et les options `CONFIG_USB_HOST_*`
fournies par défaut par ESP-IDF.

---

## Structure du code

```
src/
├── main.rs                      ← boot, USB host init, SPI slave task, polling loop
├── spi_link/                    ← protocole SPI3 (esclave)
│   ├── mod.rs
│   ├── pins.rs                  ← MOSI=12 MISO=13 SCLK=14 CS=21 READY=47
│   ├── protocol.rs              ← header 16B, Cmd enum, CRC32, magic "BK"
│   ├── transport.rs             ← driver SPI slave (DMA, ready_high/ready_low)
│   └── task.rs                  ← boucle de service des commandes (start_spi_task)
├── usb_mass_storage/            ← pile USB Host MSC
│   ├── mod.rs
│   ├── driver.rs                ← installation usb_host + msc_host, msc_event_cb
│   ├── callback.rs              ← événements MSC_DEVICE_CONNECTED/DISCONNECTED
│   ├── block_device.rs          ← bd_status / bd_read_blocks / bd_write_blocks
│   ├── fsops.rs                 ← opérations FS (montage /usb via VFS)
│   └── usb_host_msc/            ← composant C custom (FFI bindgen — module Rust `usb_msc`)
├── power_switch/                ← contrôle alim USB host (TPS)
│   ├── mod.rs
│   └── tps.rs                   ← EN_BARRE = GPIO 1, power_cycle_usb()
└── led/                         ← indicateur d'état (GPIO 8)
    ├── mod.rs
    └── led.rs                   ← LedGuard (RAII : LED ON au new, OFF au drop)
```

---

## Brochage SPI (slave)

| Signal | GPIO                                         |
|--------|----------------------------------------------|
| MOSI   | 12                                           |
| MISO   | 13                                           |
| SCLK   | 14                                           |
| CS     | 21                                           |
| READY  | 47 (sortie : signale au master « DMA prêt ») |

Croisement obligatoire avec le master :

| Master (`bindkey-tinyesp`) | Slave (ce repo)   |
|----------------------------|-------------------|
| MOSI=13                    | MOSI=12           |
| MISO=12                    | MISO=13           |
| SCLK=11                    | SCLK=14           |
| CS=10                      | CS=21             |
| READY=9 (entrée)           | READY=47 (sortie) |

---

## Repo lié

[`bindkey-tinyesp`](https://github.com/bindkey-project/bindkey-tinyesp) — firmware de la première MCU,
master USB MSC et orchestrateur cryptographique. Toute modification du
protocole SPI (taille du header, taille de payload `MAX_PAYLOAD`, séquence
handshake READY, polynôme CRC32) **doit être coordonnée** entre les deux
repos sous peine de désynchronisation immédiate au boot.

---

## Profils de build

```toml
[profile.release]
opt-level = "s"

[profile.dev]
opt-level = "z"
debug = true
```

---

## Ce que ce firmware NE fait PAS

- Aucun chiffrement n'est effectué ici : les secteurs reçus sur SPI sont
  déjà chiffrés par le master. Le slave n'est qu'un proxy USB.
- Aucune authentification biométrique ni accès à un Secure Element.
- Aucun endpoint USB device : si tu branches ce slave directement à un PC,
  il n'apparaîtra pas comme un périphérique.

---

## Licence et auteurs

Projet académique BindKey — INIZIATO William, LOPEZ Pierre-Louis, MATTEI Jean-Baptiste, ZAIETER Jassime, ADDOUH Marwa
