use std::ptr;

use esp_idf_sys::usb_msc::msc_host_event_t;

use crate::usb_mass_storage::UsbMassStorage;

/// global driver instance
static mut GLOBAL_MSC: *mut UsbMassStorage = ptr::null_mut();

pub fn set_global_mass_storage(driver: &mut UsbMassStorage){
    unsafe{
        GLOBAL_MSC = driver as *mut _;
    }
}

pub fn get_global_mass_storage() -> Option<&'static mut UsbMassStorage>{
    unsafe{
        GLOBAL_MSC.as_mut()
    }
}

/// callback function when an msc event is triggered
pub unsafe extern "C" fn msc_event_cb(event: *const msc_host_event_t, _arg: *mut core::ffi::c_void){
    if GLOBAL_MSC.is_null(){
        log::error!("MSC callback called but GLOBAL_MSC is NULL");
        return;
    }

    if event.is_null(){
        log::warn!("MSC callback received null event");
        return;
    }

    let driver = &mut *GLOBAL_MSC;
    driver.handle_raw_event(&*event);
}