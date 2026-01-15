use core::ffi::c_void;
use esp_idf_sys::*;

use crate::spi_link::SpiLink;

static mut SPI_TASK_HANDLE: TaskHandle_t = core::ptr::null_mut();

extern "C" fn spi_task_entry(arg: *mut c_void){
    let spi: &mut SpiLink = unsafe{
        &mut *(arg as *mut SpiLink)
    };
    spi.run();
}

pub fn start_spi_task(spi: &'static mut SpiLink) -> Result<(), i32>{
    unsafe{
        let name = b"spi_link\0";
        
        let stack_words: u32 = 4096;

        let prio: UBaseType_t = 10;

        let core_id: BaseType_t = 1;

        let ok = xTaskCreatePinnedToCore(
            Some(spi_task_entry),
            name.as_ptr() as *const u8,
            stack_words,
            spi as *mut _ as *mut c_void,
            prio,
            &raw mut SPI_TASK_HANDLE,
            core_id
        );

        if ok != 1{
            return Err(ESP_FAIL);
        }

        Ok(())
    }
}