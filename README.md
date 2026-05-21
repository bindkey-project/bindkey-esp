<p align="center">
  <img src="assets/logo-bindkey.png" alt="BindKey Logo" width="450"/>
</p>

<h1 align="center">BindKey</h1>

<p align="center"><i>Security at your fingertip</i></p>

---

## Project overview

**BindKey** is a hardware cybersecurity solution that resolves the trade-off between offline data security and the need for enterprise collaboration. The device sits between the host PC and a standard storage medium (USB stick, SSD, SD reader) and acts as a **legitimate Man-in-the-Middle**: every byte that flows through it is sealed and encrypted on-the-fly in **AES-256-GCM** by a secure microcontroller, and is decrypted only for users who have been **biometrically** authenticated and whose BindKey holds the access rights to the target volume. Any modification performed outside the BindKey environment makes the content unreadable. The whole system works **off-cloud**, **with no host driver**, on Windows / Linux / macOS.

### The three pillars of the project

1. **The BindKey hardware proxy** — the physical box that handles local biometric authentication, key derivation through an ATECC608A secure element, and on-the-fly encryption of the data. Made of two ESP32-S3 microcontrollers:
   - a **master** (USB MSC emulation + biometrics + AES-GCM crypto + secure element) — repo [`bindkey-tinyesp`](https://github.com/bindkey-project/bindkey-tinyesp)
   - a **slave** (drives the real physical media in USB Host mode) — repo [`bindkey-esp`](https://github.com/bindkey-project/bindkey-esp) **← this repo**
2. **A backend server (API)** — repo [`bindkey-server`](https://github.com/bindkey-project/bindkey-server) — manages users' public identities and orchestrates access delegation between BindKeys in *Zero-Knowledge* mode: only wrapped keys (ECDH-wrapped) ever travel over the network, never the plaintext volume key.
3. **The desktop software** — repo [`bindkey-software`](https://github.com/bindkey-project/bindkey-software) — Rust application that drives the BindKey through UART and provides the GUI: volume creation / deletion, sharing with a colleague, formatting, reset, and any administration operation that requires the physical presence of the key.

### Main features

- **Transparent on-the-fly encryption** — no host driver, behaves as a standard USB MSC drive
- **Local biometric authentication** by fingerprint, hardware-gated and replay-protected
- **Provable integrity** — any change made outside the BindKey makes the data unreadable (AES-GCM tag)
- **Collaborative sharing** between BindKeys of the same organization via ECDH P-256 (Zero-Knowledge)
- **Delegated enrollment** — an administrator can grant *Enroller* privilege to a team leader
- **Lifecycle management** — remote revocation, recovery-code-based restoration, wipe & reassignment
- **Tamper-evident centralized audit log** (GDPR compliance and forensic traceability)
- **Air-gapped maintenance** — secure transport of payloads to isolated systems (OT, industrial)

This repository contains the **slave** firmware (ESP32-S3 #2) — one of the two MCUs that make up the BindKey hardware proxy.

---

# bindkey-esp

BindKey - Master Project - ESP32#2 Code Repository

> Rust firmware for the BindKey **slave** — ESP32-S3 #2 of the BindKey project.

`bindkey-esp` is one of the two firmwares that make up the encrypted USB
proxy **BindKey**. This MCU sits "behind" the first one (`bindkey-tinyesp`,
see dedicated repo): it never talks directly to the host PC. Its role is
twofold:

1. **Receive commands** from the first ESP32-S3 over an SPI3 slave bus
   (sector read / write, GetStatus, GetCapacity, Flush).
2. **Drive the actual USB storage media** (USB stick, external SSD,
   SD-to-USB reader…) via a USB Host MSC stack.

From the PC's point of view, this firmware is invisible: it exposes **no**
USB endpoint to the host. Everything goes through the first MCU.

---

## Place in the BindKey architecture

```
        Host PC
            │  USB (native OS driver, MSC class)
            ▼
  ┌─────────────────────────────┐
  │   bindkey-tinyesp  (master) │
  │   ESP32-S3 #1               │
  │   • USB MSC emulation       │
  │   • Biometric auth          │
  │   • AES-GCM encryption      │
  └─────────────┬───────────────┘
                │  SPI3 inter-MCU
                ▼
  ┌─────────────────────────────┐
  │   bindkey-esp  (slave)      │   ← THIS REPO
  │   ESP32-S3 #2               │
  │   • SPI slave               │
  │   • USB Host MSC            │
  │   • Power switch management │
  └─────────────┬───────────────┘
                │  USB-OTG (host)
                ▼
        Physical USB drive
        (FAT/exFAT/NTFS/ext4)
```

The isolation between the two MCUs guarantees that the physical drive is
**unreachable** without going through the encrypted master path. No secret
(volume key, biometric template, ECDH key) ever exists on this board.

---

## Features

- **USB Host MSC** stack via `esp_usb` and a custom C component
  (`src/usb_mass_storage/usb_host_msc/`) exposing a block-device API:
  `bd_status()`, `bd_read_blocks()`, `bd_write_blocks()`
- **SPI slave** on SPI3, frame size up to 8212 bytes (16 B header + 8192 B
  payload + 4 B CRC32), commands `GetStatus`, `GetCapacity`, `Read`, `Write`,
  `Flush`
- USB host power management via a **TPS** switch (GPIO 1 = `EN_BARRE`),
  with automatic power-cycle on persistent USB errors
- Diagnostic LED (GPIO 8)
- Hotplug robustness: live USB drive reconnection, x3 retry with
  power-cycle, capacity refreshed on remount

---

## Hardware required

| Item              | Reference                                                       |
|-------------------|-----------------------------------------------------------------|
| MCU               | ESP32-S3 (any dev-board with accessible USB Host)               |
| USB Host switch   | Texas Instruments TPS (or any equivalent with an EN pin)        |
| USB-A port        | exposed on the physical-media side                              |
| Inter-MCU link    | 5 SPI3 wires + READY line from the `bindkey-tinyesp` board      |
| Storage media     | USB stick / external SSD / SD-to-USB reader, USB MSC compatible |

---

## Software prerequisites

This firmware uses the **Rust for Xtensa** toolchain maintained by Espressif,
distinct from the regular stable Rust toolchain. Same tools as for
`bindkey-tinyesp`.

### 1. Rust + `rustup`

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. `espup` (Xtensa + ESP-IDF toolchain installer)

```bash
cargo install espup --locked
espup install
. $HOME/export-esp.sh        # to source in every shell
```

`espup install` installs the patched `rustc` compiler for Xtensa, GCC
Xtensa, LLVM, and configures the environment variables required by
`esp-idf-sys` (which will call the ESP-IDF C build in the background).

### 3. Flash and scaffolding tools

```bash
cargo install ldproxy           # linker proxy used by esp-idf-sys
cargo install espflash --locked # flash + serial monitor
cargo install cargo-generate    # to create new ESP-IDF projects
```

### 4. (Optional) Bootstrapping a new ESP-IDF + Rust project from scratch

If you want to regenerate a skeleton equivalent to this repo:

```bash
cargo generate esp-rs/esp-idf-template cargo
```

Answers to give to the template:
- **MCU**: `esp32s3`
- **ESP-IDF version**: `v5.3.3` (see `.cargo/config.toml` in this repo)
- **STD support**: `true` (used here)

### System dependencies (Linux/macOS)

```bash
# Debian/Ubuntu
sudo apt install -y git wget flex bison gperf python3 python3-pip python3-venv \
                    cmake ninja-build ccache libffi-dev libssl-dev dfu-util \
                    libusb-1.0-0 pkg-config
```

---

## Pinned versions (extract from `Cargo.toml`)

| Item             | Version                                       |
|------------------|-----------------------------------------------|
| Rust edition     | 2021                                          |
| `rust-version`   | ≥ 1.77                                        |
| Rust toolchain   | `channel = "esp"` (see `rust-toolchain.toml`) |
| Target           | `xtensa-esp32s3-espidf`                       |
| ESP-IDF          | `v5.3.3`                                      |
| `esp-idf-svc`    | `0.51`                                        |
| `esp-idf-sys`    | `0.36` (feature `native`)                     |
| `embuild`        | `0.33`                                        |

Custom C component embedded: `src/usb_mass_storage/usb_host_msc/` (bindings
generated into the Rust module `usb_msc` via `bindgen`).

---

## Build, flash, monitor

```bash
git clone https://github.com/bindkey-project/bindkey-esp.git
cd bindkey-esp

# Activate the ESP toolchain
. $HOME/export-esp.sh

# Debug build
cargo build

# Release build
cargo build --release

# Build + flash + serial monitor (main command)
cargo run

# Lint
cargo clippy -- -D warnings
```

> ⚠️ Same as for `bindkey-tinyesp`: **always** use `cargo build` /
> `cargo run`. Do not invoke `idf.py build` directly — `esp-idf-sys`
> orchestrates the full ESP-IDF build.

---

## `sdkconfig.defaults` configuration

The structural entries for this slave:

```ini
CONFIG_ESP_MAIN_TASK_STACK_SIZE=24576
CONFIG_FREERTOS_HZ=1000

# Long filenames support on FAT (useful for FAT32/exFAT drives)
CONFIG_FATFS_LFN_HEAP=y
CONFIG_FATFS_MAX_LFN=255
```

The USB Host MSC stack is enabled through the embedded component
`src/usb_mass_storage/usb_host_msc/` and the `CONFIG_USB_HOST_*` options
provided by default by ESP-IDF.

---

## Source tree

```
src/
├── main.rs                      ← boot, USB host init, SPI slave task, polling loop
├── spi_link/                    ← slave-side SPI3 protocol
│   ├── mod.rs
│   ├── pins.rs                  ← MOSI=12 MISO=13 SCLK=14 CS=21 READY=47
│   ├── protocol.rs              ← 16B header, Cmd enum, CRC32, magic "BK"
│   ├── transport.rs             ← SPI slave driver (DMA, ready_high/ready_low)
│   └── task.rs                  ← command service loop (start_spi_task)
├── usb_mass_storage/            ← USB Host MSC stack
│   ├── mod.rs
│   ├── driver.rs                ← usb_host + msc_host install, msc_event_cb
│   ├── callback.rs              ← MSC_DEVICE_CONNECTED/DISCONNECTED events
│   ├── block_device.rs          ← bd_status / bd_read_blocks / bd_write_blocks
│   ├── fsops.rs                 ← FS operations (mount /usb via VFS)
│   └── usb_host_msc/            ← custom C component (FFI bindgen — Rust module `usb_msc`)
├── power_switch/                ← USB host power control (TPS)
│   ├── mod.rs
│   └── tps.rs                   ← EN_BARRE = GPIO 1, power_cycle_usb()
└── led/                         ← status indicator (GPIO 8)
    ├── mod.rs
    └── led.rs                   ← LedGuard (RAII: LED ON at new, OFF at drop)
```

---

## SPI pinout (slave)

| Signal | GPIO                                              |
|--------|---------------------------------------------------|
| MOSI   | 12                                                |
| MISO   | 13                                                |
| SCLK   | 14                                                |
| CS     | 21                                                |
| READY  | 47 (output: signals "DMA ready" to the master)    |

Mandatory cross-wiring with the master:

| Master (`bindkey-tinyesp`) | Slave (this repo)  |
|----------------------------|--------------------|
| MOSI=13                    | MOSI=12            |
| MISO=12                    | MISO=13            |
| SCLK=11                    | SCLK=14            |
| CS=10                      | CS=21              |
| READY=9 (input)            | READY=47 (output)  |

---

## Related repo

[`bindkey-tinyesp`](https://github.com/bindkey-project/bindkey-tinyesp) —
firmware for the first MCU, USB MSC master and cryptographic orchestrator.
Any change to the SPI protocol (header size, `MAX_PAYLOAD` payload size,
READY handshake sequence, CRC32 polynomial) **must be coordinated** between
the two repos, otherwise the link desynchronizes immediately at boot.

---

## Build profiles

```toml
[profile.release]
opt-level = "s"

[profile.dev]
opt-level = "z"
debug = true
```

---

## What this firmware does NOT do

- No encryption is performed here: the sectors received over SPI are
  already encrypted by the master. The slave is only a USB proxy.
- No biometric authentication, no access to a Secure Element.
- No USB device endpoint: if you plug this slave directly into a PC,
  it will not appear as a peripheral.

---

## License and authors

BindKey academic project — INIZIATO William, LOPEZ Pierre-Louis, MATTEI Jean-Baptiste, ZAIETER Jassime, ADDOUH Marwa
