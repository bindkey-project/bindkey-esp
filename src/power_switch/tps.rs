use esp_idf_sys::*;
use std::sync::atomic::{AtomicBool, Ordering};

const EN_BARRE_GPIO: i32 = 1;

static TPS_INITIALIZED: AtomicBool = AtomicBool::new(false);

pub struct TPSguard;

impl TPSguard{
    pub fn new() -> Self{
        unsafe{
            gpio_reset_pin(EN_BARRE_GPIO);
            gpio_set_direction(EN_BARRE_GPIO, gpio_mode_t_GPIO_MODE_OUTPUT);
            gpio_set_level(EN_BARRE_GPIO, 0);
        }
        Self
    }
}

pub fn set_global_tps(_tps: &TPSguard){
    TPS_INITIALIZED.store(true, Ordering::Release);
}

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
