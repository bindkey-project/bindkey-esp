mod usb_mass_storage;
mod spi_link;
mod led;
mod power_switch;

use crate::usb_mass_storage::*;
use crate::spi_link::*;
use crate::task::start_spi_task;
use crate::led::*;
use crate::power_switch::*;

fn main() {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let _led = LedGuard::new();

    let _tps = TPSguard::new();
    set_global_tps(&_tps);

    let mut usb = UsbMassStorage::new();

    set_global_mass_storage(&mut usb);

    if let Err(e) = UsbMassStorage::init_host(){
        log::error!("USB init failed: {}", e);
        return;
    }

    log::info!("USB Host initialized.");

    let spi_static: &'static mut SpiLink = Box::leak(Box::new(SpiLink::new()));

    if let Err(e) = SpiLink::init(spi_static){
        log::error!("SPI init failed: {}", e);
        return;
    }
    log::info!("SPI Link Slave initialized.");

    if let Err(e) = start_spi_task(spi_static){
        log::error!("Failed to start SPI task: {}", e);
        return;
    }
    log::info!("SPI task started");

    loop{
        usb.poll();

        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
