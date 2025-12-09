use esp_idf_svc::sys::*;
use esp_idf_sys::usb_msc::{
    esp_vfs_fat_mount_config_t,
    msc_host_device_handle_t,
    msc_host_driver_config_t,
    msc_host_event_t,
    msc_host_event_t_MSC_DEVICE_CONNECTED,
    msc_host_event_t_MSC_DEVICE_DISCONNECTED,
    msc_host_handle_events,
    msc_host_install,
    msc_host_install_device,
    msc_host_uninstall_device,
    msc_host_vfs_handle_t,
    msc_host_vfs_register,
    msc_host_vfs_unregister,
};

use esp_idf_svc::sys::esp_err_to_name;
use std::ffi::CString;
use std::path::Path;

/// path to mount the usb hard drive with the ESP32
const MNT_PATH: &str = "/usb";      

/// number of retries if a problem occurs
const RETRY_COUNT: i32 = 3;                                                                    

/// struct to define the usb mass storage device with its handlers and address
pub struct UsbMassStorage{
    /// USB address of the device
    pending_addr: i32,   

    /// number of retries left in case of an error occuring
    retries_left: i32, 

    /// handle to the currently opened MSC device                                                                         
    handle: msc_host_device_handle_t,             

    /// handle to the mounted VFS instance                                              
    vfs: msc_host_vfs_handle_t,                                                                 
}

impl UsbMassStorage{
    /// init of a new mass storage device
    pub fn new() -> Self{
        unsafe{
            UsbMassStorage{
                pending_addr: -1,
                retries_left: 0,
                handle: core::ptr::null_mut(),
                vfs: core::ptr::null_mut(),
            }
        }
    }

    pub fn init_host() -> Result<(), i32>{
        unsafe{
            let host_cfg = usb_host_config_t{
                skip_phy_setup: false,
                root_port_unpowered: false,
                intr_flags: ESP_INTR_FLAG_LEVEL1 as i32,
                enum_filter_cb: None,
            };

            let err = usb_host_install(&host_cfg);
            if err != ESP_OK{
                log::error!("usb_host_install failed : {}", err);
                return Err(err);
            }

            log::info!("USB Host installed.");

            let msc_cfg = msc_host_driver_config_t{
                task_priority: 5,
                stack_size: 4096,
                core_id: 0,
                callback: Some(crate::usb_mass_storage::callback::msc_event_cb),
                callback_arg: core::ptr::null_mut(),
                create_backround_task: true,
            };

            let ret = msc_host_install(&msc_cfg);
            if ret != ESP_OK{
                log::error!("msc_host_install failed : {}", ret);
                return Err(ret);
            }
            
            log::info!("MSC driver installed.");

            Ok(())
        }
    }

    /// function to verify if the device is mounted
    pub fn is_mounted(&self) -> bool{
        !self.vfs.is_null()
    }

    pub fn handle_raw_event(&mut self, event: &msc_host_event_t){
        match event.event{
            e if e == msc_host_event_t_MSC_DEVICE_CONNECTED => {
                let addr = unsafe { event.device.address };
                log::info!("MSC CONNECTED, addr = {}", addr);

                self.pending_addr = addr as i32;
                self.retries_left = RETRY_COUNT;

                self.handle = core::ptr::null_mut();
                self.vfs = core::ptr::null_mut();
            }

            e if e == msc_host_event_t_MSC_DEVICE_DISCONNECTED => {
                log::info!("MSC DISCONNECTED");

                self.unmount_vfs();
                self.close_device();
                self.pending_addr = -1;
                self.retries_left = 0;
            }

            other => {
                log::info!("MSC event : {}", other);
            }
        }
    }

    /// function to be called in the main.rs file in order to make the usb mass storage service available
    pub fn poll(&mut self){
        unsafe{
            let mut flags: u32 = 0;

            // read USB Host stack events
            usb_host_lib_handle_events(10, &mut flags);

            // handling MSC events
            msc_host_handle_events(10);

            /// 
            self.try_open_and_mount();
        }
    }

    fn try_open_and_mount(&mut self){
        if self.pending_addr < 0 || self.retries_left <= 0 {
            return;
        }

        let addr = self.pending_addr as u8;

        log::info!("Try open MSC @ addr={} ({} retries left)", addr, self.retries_left);

        let mut handle: msc_host_device_handle_t = core::ptr::null_mut();
        let ret = unsafe{msc_host_install_device(addr, &mut handle)};
        let name = unsafe{
            let ptr = esp_err_to_name(ret);
            std::ffi::CStr::from_ptr(ptr).to_string_lossy()
        };
        log::info!("msc_host_install_device returned {} ({})", ret, name);

        if ret == ESP_OK{
            self.handle = handle;
            if self.mount_vfs(){
                self.pending_addr = -1;
                self.retries_left = 0;
            }
            else{
                self.close_device();
                self.retries_left -= 1; 
            }
        }
        else if ret == ESP_ERR_INVALID_STATE{
            log::warn!("INVALID STATE -> retrying later");
            self.retries_left -= 1;
        }
        else{
            log::error!("Fatal MSC error -> abort retries");
            self.pending_addr -= 1;
            self.retries_left = 0;
        }
    }

    fn close_device(&mut self){
        if !self.handle.is_null(){
            unsafe{msc_host_uninstall_device(self.handle)};
            self.handle = core::ptr::null_mut();
        }
    }

    fn mount_vfs(&mut self) -> bool{
        let mount_cfg = esp_vfs_fat_mount_config_t{
            format_if_mount_failed: false,
            max_files: 8,
            allocation_unit_size: 8192,
            disk_status_check_enable: true,
            use_one_fat: true,
        };

        let base_path = CString::new(MNT_PATH).unwrap();
        let mut vfs_handle: msc_host_vfs_handle_t = core::ptr::null_mut();

        let vfs_ret = unsafe{
            msc_host_vfs_register(self.handle, base_path.as_ptr(), &mount_cfg as *const _, &mut vfs_handle)
        };

        let err_name = unsafe{std::ffi::CStr::from_ptr(esp_err_to_name(vfs_ret)).to_string_lossy()};

        if vfs_ret == ESP_OK{
            self.vfs = vfs_handle;
            log::info!("VFS mounted at {}", MNT_PATH);

            self.ls_all_device(Path::new(MNT_PATH));
            true
        }
        else{
            log::error!("VFS MOUNT FAILED: {} ({})", vfs_ret, err_name);
            false
        }
    }

    fn unmount_vfs(&mut self){
        if !self.vfs.is_null(){
            unsafe{ msc_host_vfs_unregister(self.vfs) };
            self.vfs = core::ptr::null_mut();
        }
    }

}
