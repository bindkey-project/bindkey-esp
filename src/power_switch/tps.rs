use esp_idf_sys::*;

const EN_BARRE_GPIO: i32 = 1;

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

