use esp_idf_svc::sys::*;
use esp_idf_sys::usb_msc::{
    scsi_cmd_read_capacity,
    scsi_cmd_read10,
    scsi_cmd_write10,
    scsi_cmd_unit_ready
};

use crate::usb_mass_storage::driver::UsbMassStorage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockDevStatus {
    NotPresent,
    NotReady,
    Ready,
}

impl UsbMassStorage {
    pub fn bd_status(&self) -> BlockDevStatus {
        if self.handle.is_null(){
            return BlockDevStatus::NotPresent;
        }

        if self.block_size == 0 || self.block_count == 0{
            return BlockDevStatus::NotReady;
        }

        let err = unsafe { scsi_cmd_unit_ready(self.handle) };
        if err == ESP_OK{
            return BlockDevStatus::Ready;
        }
        else{
            return BlockDevStatus::NotReady;
        }
    }

    pub fn bd_refresh_capacity(&mut self) -> Result<(u32, u32), i32>{
        if self.handle.is_null(){
            return Err(ESP_ERR_INVALID_STATE);
        }

        let mut bs: u32 = 0;
        let mut bc: u32 = 0;

        let err = unsafe { scsi_cmd_read_capacity(self.handle, &mut bs, &mut bc) };
        if err != ESP_OK{
            return Err(err);
        }

        self.block_count = bc;
        self.block_size = bs;

        Ok((bs, bc))
    }

    pub fn bd_read_blocks(&self, lba: u32, nblocks: u32, out: &mut [u8]) -> Result<(), i32>{
        if self.handle.is_null(){
            return Err(ESP_ERR_INVALID_STATE);
        }

        if nblocks == 0{
            return Err(ESP_ERR_INVALID_ARG);
        }

        if self.block_size == 0 || self.block_count == 0{
            return Err(ESP_ERR_INVALID_STATE);
        }

        let end = lba.saturating_add(nblocks);
        if end > self.block_count{
            return Err(ESP_ERR_INVALID_ARG);
        }

        let need = (nblocks as usize) * (self.block_size as usize);
        if out.len() != need {
            return Err(ESP_ERR_INVALID_ARG);
        }

        let err = unsafe { scsi_cmd_read10(self.handle, out.as_mut_ptr(), lba, nblocks, self.block_size) };
        if err == ESP_OK{
            Ok(())
        }
        else{
            Err(err)
        }

    }

    pub fn bd_write_blocks(&self, lba: u32, nblocks: u32, data: &[u8]) -> Result<(), i32>{
        if self.handle.is_null(){
            return Err(ESP_ERR_INVALID_STATE);
        }

        if nblocks == 0{
            return Err(ESP_ERR_INVALID_ARG);
        }

        if self.block_size == 0 || self.block_count == 0{
            return Err(ESP_ERR_INVALID_STATE);
        }

        let end = lba.saturating_add(nblocks);
        if end > self.block_count{
            return Err(ESP_ERR_INVALID_ARG);
        }

        let need = (nblocks as usize) * (self.block_size as usize);
        if data.len() != need{
            return Err(ESP_ERR_INVALID_ARG);
        }

        let err = unsafe { scsi_cmd_write10(self.handle, data.as_ptr(), lba, nblocks, self.block_size)};

        if err == ESP_OK{
            Ok(())
        }
        else{
            Err(err)
        }
    }

    // à compléter si besoin
    pub fn bd_flush(&self) -> Result<(), i32> {
        if self.handle.is_null() {
            return Err(ESP_ERR_INVALID_STATE);
        }
        Ok(())
    }


}