use std::io;
use std::path::Path;
use std::fs::read_dir;

use crate::usb_mass_storage::UsbMassStorage;

impl UsbMassStorage{
    /// list all files in the device
    pub fn ls_all_device(&self, path: &Path) -> io::Result<()>{
        log::info!("Listing: {}", path.display());

        // iterate through directory entries
        for entry in read_dir(path)?{
            let entry = entry?;
            let subpath = entry.path();

            log::info!(" - {}", subpath.display());

            if subpath.is_dir(){
                self.ls_all_device(&subpath)?;
            }
        }

        Ok(())
    }
}