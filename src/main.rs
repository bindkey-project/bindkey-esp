mod usb_mass_storage;
mod spi_link;

use crate::usb_mass_storage::*;
use crate::spi_link::*;

fn main() {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("Hello, world!");

    let mut usb = UsbMassStorage::new();

    set_global_mass_storage(&mut usb);

    if let Err(e) = UsbMassStorage::init_host(){
        log::error!("USB init failed: {}", e);
        return;
    }

    log::info!("USB Host initialized.");

    //uniquement test
    let mut spi = SpiLink::new();
    if let Err(e) = SpiLink::init(&mut spi){
        log::error!("SPI init failed: {}", e);
        return;
    }

    log::info!("SPI Link Slave initialized.");
    spi.run();

    /*loop{
        //usb.poll();
       

        std::thread::sleep(std::time::Duration::from_millis(100));
    }*/
}
