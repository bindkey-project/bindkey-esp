use esp_idf_sys::*;
use std::sync::atomic::{AtomicBool, Ordering};

// GPIO driving the TPS load-switch enable line (power to the downstream USB drive)
const EN_BARRE_GPIO: i32 = 1;

// guards power_cycle_usb() until the TPS has been set up
static TPS_INITIALIZED: AtomicBool = AtomicBool::new(false);

// RAII guard that configures the TPS enable GPIO at boot
pub struct TPSguard;

impl TPSguard{
    // configures the enable GPIO as output and sets its default level
    pub fn new() -> Self{
        unsafe{
            gpio_reset_pin(EN_BARRE_GPIO);
            gpio_set_direction(EN_BARRE_GPIO, gpio_mode_t_GPIO_MODE_OUTPUT);
            gpio_set_level(EN_BARRE_GPIO, 0);
        }
        Self
    }
}

// marks the TPS as initialized so power cycling is allowed
pub fn set_global_tps(_tps: &TPSguard){
    TPS_INITIALIZED.store(true, Ordering::Release);
}

// power-cycles the downstream USB drive by toggling the TPS enable line (~2s off)
pub fn power_cycle_usb(){
    if !TPS_INITIALIZED.load(Ordering::Acquire){
        return;
    }
    log::warn!("USB: power cycling USB...");
    unsafe{
        gpio_set_level(EN_BARRE_GPIO, 1);
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
    unsafe{
        gpio_set_level(EN_BARRE_GPIO, 0);
    }
}
