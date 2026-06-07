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

use crate::power_switch::power_cycle_usb;

// path to mount the usb hard drive with the ESP32
const MNT_PATH: &str = "/usb";      

// number of retries if a problem occurs
const RETRY_COUNT: i32 = 3;

// flag to allow debugging with a vfs mounting
const DEBUG_VFS: i32 = 0;

// struct to define the usb mass storage device with its handlers and address
pub struct UsbMassStorage{
    // USB address of the device
    pub(crate) pending_addr: i32,   

    // number of retries left in case of an error occuring
    pub(crate) retries_left: i32, 

    // handle to the currently opened MSC device                                                                         
    pub(crate) handle: msc_host_device_handle_t,             

    // handle to the mounted VFS instance                                              
    pub(crate) vfs: msc_host_vfs_handle_t,  

    pub(crate) block_size: u32,
    pub(crate) block_count: u32,  

    pub(crate) no_device_since: Option<std::time::Instant>                                                            
}

impl UsbMassStorage{
    // init of a new mass storage device
    pub fn new() -> Self{
        UsbMassStorage{
            pending_addr: -1,
            retries_left: 0,
            handle: core::ptr::null_mut(),
            vfs: core::ptr::null_mut(),
            block_size: 0,
            block_count: 0,
            no_device_since: None
        }
    }

    // host initialisation, before trying to detect the usb hard drive
    pub fn init_host() -> Result<(), i32>{
        std::thread::sleep(std::time::Duration::from_millis(500));
        unsafe{
            let host_cfg = usb_host_config_t{
                skip_phy_setup: false,                   // let ESP-IDF configure automatically the PHY USB 
                root_port_unpowered: false,              // USB port powers the USB device
                intr_flags: ESP_INTR_FLAG_LEVEL1 as i32, // interruption flag, low priority
                enum_filter_cb: None,                    // no complementary enum callback
            };

            // install the USB Host driver
            let err = usb_host_install(&host_cfg);
            if err != ESP_OK{
                log::error!("usb_host_install failed : {}", err);
                return Err(err);
            }

            log::info!("USB Host installed.");
            std::thread::sleep(std::time::Duration::from_millis(200));

            // mass storage class (msc) host driver configuration
            let msc_cfg = msc_host_driver_config_t{
                task_priority: 5,                                                // priority of the msc background task
                stack_size: 4096,                                                // stack size allocated for the msc task
                core_id: 0,                                                      // CPU core on which the msc task will run
                callback: Some(crate::usb_mass_storage::callback::msc_event_cb), // callback function triggered on msc events (connect/disconnect)
                callback_arg: core::ptr::null_mut(),                             // no custom argument for the callback
                create_backround_task: true,                                     // automatically create the background msc handling task
            };

            std::thread::sleep(std::time::Duration::from_millis(100));

            // install the msc host driver
            let ret = msc_host_install(&msc_cfg);
            if ret != ESP_OK{
                log::error!("msc_host_install failed : {}", ret);
                return Err(ret);
            }
            
            log::info!("MSC driver installed.");

            Ok(())
        }
    }

    // function to verify if the device is mounted
    pub fn is_mounted(&self) -> bool{
        !self.vfs.is_null()
    }

    // handles raw msc events received from the USB Host stack => reacts to device connection, disconnection events and updates the internal state accordingly
    pub fn handle_raw_event(&mut self, event: &msc_host_event_t){
        match event.event{
            // the usb mass storage device has been connected
            e if e == msc_host_event_t_MSC_DEVICE_CONNECTED => {
                // get the usb device address during enumeration
                let addr = unsafe { event.device.address };
                log::info!("MSC CONNECTED, addr = {}", addr);
                
                // store the pending device address
                self.pending_addr = addr as i32;
                // reset retry counter for device opening attempts
                self.retries_left = RETRY_COUNT;

                // reset device handle and vfs pointer  
                self.handle = core::ptr::null_mut();
                self.vfs = core::ptr::null_mut();

                // reset block parameters
                self.block_size = 0;
                self.block_count = 0;
            }

            // the usb mass storage device has been disconnected   
            e if e == msc_host_event_t_MSC_DEVICE_DISCONNECTED => {
                log::info!("MSC DISCONNECTED");
                
                // unmount the virtual file system 
                self.unmount_vfs();
                
                // close the msc device handle and release associated resources
                self.close_device();
                
                // reset internal state
                self.pending_addr = -1;
                self.retries_left = 0;

                // reset block parameters
                self.block_size = 0;
                self.block_count = 0;

            }

            other => {
                log::info!("MSC event : {}", other);
            }
        }
    }

    // function to be called in the main.rs file in order to make the usb mass storage service available
    pub fn poll(&mut self){
        unsafe{
            let mut flags: u32 = 0;

            // read USB Host stack events
            usb_host_lib_handle_events(10, &mut flags);

            // handling MSC events
            msc_host_handle_events(10);

            // attempt to open the msc device and mount its filesystem
            self.try_open_and_mount();

            if self.handle.is_null() && self.pending_addr < 0{
                let now = std::time::Instant::now();
                match self.no_device_since{
                    None => self.no_device_since = Some(now),
                    Some(since) => {
                        if since.elapsed().as_secs() >= 2{
                            log::warn!("USB watchdog: no device for 2s, power cycling...");
                            power_cycle_usb();
                            self.no_device_since = Some(std::time::Instant::now());
                        }
                    }
                }
            }
            else{
                self.no_device_since = None;
            }
        }
    }

    // attempts to open a MSC device and mount a filesystem
    fn try_open_and_mount(&mut self){
        // abort if there is no pending device or no retries left
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
            self.pending_addr = -1;
            self.retries_left = 0;

            match self.bd_refresh_capacity(){
                Ok((bs, bc)) => log::info!("MSC capacity: blocks={} block_size={}", bc, bs),                                                                                                                                              
                Err(e) => log::warn!("bd_refresh_capacity failed: {}", e),                                                                                                                                                                
            }  

            if DEBUG_VFS == 1{
                if !self.mount_vfs(){
                    self.close_device();
                    self.retries_left -= 1; 
                }
            }
        }
        else if ret == ESP_ERR_INVALID_STATE{
            log::warn!("INVALID STATE -> retrying later");
            self.retries_left -= 1;
        }
        else{
            log::error!("Fatal MSC error -> abort retries");
            self.pending_addr = -1;
            self.retries_left = 0;
        }
    }

    // uninstalls the MSC device and clears the handle
    fn close_device(&mut self){
        if !self.handle.is_null(){
            unsafe{msc_host_uninstall_device(self.handle)};
            self.handle = core::ptr::null_mut();
        }
    }

    // mounts the FAT VFS at MNT_PATH (debug only; lists the contents on success)
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

            let ls_result_debug = self.ls_all_device(Path::new(MNT_PATH));
            match ls_result_debug {
                Ok(()) => log::info!("-- ls successfully done --"),
                Err(e) => log::error!("-- ls error {} --", e)
            }
            true
        }
        else{
            log::error!("VFS MOUNT FAILED: {} ({})", vfs_ret, err_name);
            false
        }
    }

    // unmounts the FAT VFS and clears the handle
    fn unmount_vfs(&mut self){
        if !self.vfs.is_null(){
            unsafe{ msc_host_vfs_unregister(self.vfs) };
            self.vfs = core::ptr::null_mut();
        }
    }

}
