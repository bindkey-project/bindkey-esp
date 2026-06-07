use esp_idf_sys::*;

const LED_GPIO: i32 = 8;

pub struct LedGuard;

// turning on the LED by default
impl LedGuard{
    pub fn new() -> Self{
        unsafe{
            gpio_reset_pin(LED_GPIO);
            gpio_set_direction(LED_GPIO, gpio_mode_t_GPIO_MODE_OUTPUT);
            gpio_set_level(LED_GPIO, 1);
        }
        Self
    }
}

// properly turning off the LED
impl Drop for LedGuard{
    fn drop(&mut self){
        unsafe{
            gpio_set_level(LED_GPIO, 0);
        }
    }
}