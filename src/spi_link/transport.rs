use core::ptr;
use esp_idf_sys::*;

use super::pins::*;
use super::protocol::*;
use crate::usb_mass_storage::UsbMassStorage;
use crate::usb_mass_storage::block_device::*;
use crate::usb_mass_storage::get_global_mass_storage;

#[derive(Default)]
struct PerfSlave {
    read_ops: u64,
    write_ops: u64,
    read_usb_us: u64,
    write_usb_us: u64,
    read_total_us: u64,
    write_total_us: u64
}

impl PerfSlave {
    fn log_if_needed(&self){
        let n = self.read_ops + self.write_ops;
        if n == 0 || (n % 1024) != 0 { return; }

        let r_avg_total = if self.read_ops > 0 { (self.read_total_us / self.read_ops) as u32 } else { 0 };
        let r_avg_usb   = if self.read_ops > 0 { (self.read_usb_us   / self.read_ops) as u32 } else { 0 };

        let w_avg_total = if self.write_ops > 0 { (self.write_total_us / self.write_ops) as u32 } else { 0 };
        let w_avg_usb   = if self.write_ops > 0 { (self.write_usb_us   / self.write_ops) as u32 } else { 0 };

        log::info!(
            "SLAVE PROF: R ops={} total_us={} usb_us={} | W ops={} total_us={} usb_us={}",
            self.read_ops, r_avg_total, r_avg_usb,
            self.write_ops, w_avg_total, w_avg_usb
        );
    }
}



struct DmaBuf{
    ptr: *mut u8,
    len: usize
}

impl DmaBuf{
    fn new(len: usize) -> Self{
        unsafe{
            let p = heap_caps_malloc(len, (MALLOC_CAP_DMA | MALLOC_CAP_8BIT) as u32) as *mut u8;
            if p.is_null(){
                panic!("heap_caps_malloc(DMA) failed for {} bytes", len);
            }
            core::ptr::write_bytes(p, 0u8, len);
            Self{
                ptr: p,
                len: len
            }
        }
    }

    #[inline]
    fn as_ptr(&self) -> *const u8{
        self.ptr as *const u8
    }

    #[inline]
    fn as_mut_ptr(&mut self) -> *mut u8{
        self.ptr
    }

    #[inline]
    fn as_slice(&self) -> &[u8]{
        unsafe{
            core::slice::from_raw_parts(self.ptr as *const u8, self.len)
        }
    }

    #[inline]
    fn as_mut_slice(&mut self) -> &mut [u8]{
        unsafe{
            core::slice::from_raw_parts_mut(self.ptr, self.len)
        }
    }

    #[inline]
    fn clear(&mut self){
        unsafe{
            core::ptr::write_bytes(self.ptr, 0u8, self.len)
        }
    }
}

impl Drop for DmaBuf{
    fn drop(&mut self){
        unsafe{
            heap_caps_free(self.ptr as *mut core::ffi::c_void);
        }
    }
}

const HDR_LEN: usize = core::mem::size_of::<Header>();
const RX_LEN: usize = HDR_LEN + MAX_PAYLOAD + CRC_LEN;
pub struct SpiLink{
    inited: bool,
    //2-phase storage
    resp_len: usize,
    resp_buf: [u8; HDR_LEN + MAX_PAYLOAD + CRC_LEN],

    block_buf: [u8; MAX_PAYLOAD],

    perf: PerfSlave,
    last_req_done_us: i64
}

impl SpiLink{
    pub fn new() -> Self{
        Self{
            inited: false,
            resp_len: 0,
            resp_buf: [0u8; RX_LEN],
            block_buf: [0u8; MAX_PAYLOAD],
            perf: PerfSlave::default(),
            last_req_done_us: 0
        }
    }

    pub fn init(&mut self) -> Result<(), i32>{
        unsafe{
            gpio_reset_pin(PIN_READY as gpio_num_t);
            gpio_set_direction(PIN_READY as gpio_num_t, gpio_mode_t_GPIO_MODE_OUTPUT);
            gpio_set_level(PIN_READY as gpio_num_t, 0);

            let mut buscfg: spi_bus_config_t = core::mem::zeroed();
            buscfg.__bindgen_anon_1.mosi_io_num = PIN_MOSI;
            buscfg.__bindgen_anon_2.miso_io_num = PIN_MISO;
            buscfg.sclk_io_num = PIN_SCLK;
            buscfg.__bindgen_anon_3.quadwp_io_num = -1;
            buscfg.__bindgen_anon_4.quadhd_io_num = -1;
            buscfg.data4_io_num = -1;
            buscfg.data5_io_num = -1;
            buscfg.data6_io_num = -1;
            buscfg.data7_io_num = -1;
            buscfg.max_transfer_sz = RX_LEN as i32; // 528 bytes


            let mut slvcfg: spi_slave_interface_config_t = core::mem::zeroed();
            slvcfg.spics_io_num = PIN_CS;
            slvcfg.queue_size = 1;
            slvcfg.mode = 0;

            let dma_chan = spi_common_dma_t_SPI_DMA_CH_AUTO;

            let err = spi_slave_initialize(
                spi_host_device_t_SPI3_HOST,
                &buscfg,
                &slvcfg,
                dma_chan,
            );

            if err != ESP_OK {
                return Err(err);
            }

            self.inited = true;

            Ok(())
        }
    }

    fn arm_response_with_arg1(&mut self, req: &Header, status: i32, arg1: u32, payload: &[u8]){
        let mut resp_hdr = Header::response_for(req, status);
        resp_hdr.reserved = req.reserved;
        resp_hdr.arg1 = arg1;

        let hdr_bytes = unsafe{
            core::slice::from_raw_parts(
                (&resp_hdr as *const Header) as *const u8,
                HDR_LEN,
            )
        };

        self.resp_buf[..HDR_LEN].copy_from_slice(hdr_bytes);

        let max_pay = RX_LEN - HDR_LEN;
        let pay_len = core::cmp::min(payload.len(), max_pay);
        if pay_len > 0{
            self.resp_buf[HDR_LEN..HDR_LEN + pay_len].copy_from_slice(&payload[..pay_len]);
            self.resp_len = HDR_LEN + pay_len;
        }
        else{
            self.resp_len = HDR_LEN;
        }

        //log::info!("spi_link: armed response len={}, arg1={}", self.resp_len, arg1);

    }

    fn arm_response_from_block_buf(&mut self, req: &Header, status: i32, arg1: u32, len: usize){
        let mut resp_hdr = Header::response_for(req, status);
        resp_hdr.reserved = req.reserved;
        resp_hdr.arg1 = arg1;

        // header
        let hdr_bytes = unsafe{
            core::slice::from_raw_parts((&resp_hdr as *const Header) as *const u8, HDR_LEN)
        };
        self.resp_buf[..HDR_LEN].copy_from_slice(hdr_bytes);

        // payload from self.block_buf + CRC32 trailer
        let pay_len = core::cmp::min(len, MAX_PAYLOAD);
        if pay_len > 0 {
            self.resp_buf[HDR_LEN..HDR_LEN + pay_len].copy_from_slice(&self.block_buf[..pay_len]);
            let crc = spi_crc32(&self.block_buf[..pay_len]);
            self.resp_buf[HDR_LEN + pay_len..HDR_LEN + pay_len + CRC_LEN].copy_from_slice(&crc.to_le_bytes());
            self.resp_len = HDR_LEN + pay_len + CRC_LEN;
        }
        else{
            self.resp_len = HDR_LEN;
        }
    }


    #[inline]
    fn arm_response(&mut self, req: &Header, status: i32, payload: &[u8]){
        self.arm_response_with_arg1(req, status, 0, payload);
    }

    #[inline]
    fn ready_high(){
        unsafe{
            gpio_set_level(PIN_READY as gpio_num_t, 1);
        }
    }

    #[inline]
    fn ready_low(){
        unsafe{
            gpio_set_level(PIN_READY as gpio_num_t, 0);
        }
    }

    const SPI_XFER_TIMEOUT_TICKS: u32 = u32::MAX;
    fn spi_slave_xfer(rx_dma: &mut DmaBuf, tx_dma: &DmaBuf, nbytes: usize) -> bool{
        let mut t: spi_slave_transaction_t = unsafe{core::mem::zeroed()};
        t.length = nbytes * 8;
        t.rx_buffer = rx_dma.as_mut_ptr() as *mut _;
        t.tx_buffer = tx_dma.as_ptr() as *const _;
        t.user = ptr::null_mut();
        let err = unsafe{
            spi_slave_transmit(spi_host_device_t_SPI3_HOST, &mut t, Self::SPI_XFER_TIMEOUT_TICKS)
        };
        if err == ESP_ERR_TIMEOUT{
            log::warn!("spi_slave_xfer: timeout - resync, retour au header");
            return false;
        }
        if err != ESP_OK{
            log::error!("spi_slave_transmit err={}", err);
            return false;
        }
        true
    }

    fn send_response(&mut self, rx_dma: &mut DmaBuf, tx_dma: &mut DmaBuf){
        let n = self.resp_len;
        tx_dma.as_mut_slice()[..n].copy_from_slice(&self.resp_buf[..n]);
        Self::ready_high();
        Self::spi_slave_xfer(rx_dma, tx_dma, n);
        Self::ready_low();
    }

    fn ensure_capacity_known(usb: &mut UsbMassStorage){
        if usb.block_size == 0 || usb.block_count == 0{
            let _ = usb.bd_refresh_capacity();
        }
    }

    fn compute_chunk(usb: &UsbMassStorage, lba_start: u32, nblocks_total: u32, chunk_idx: u16) -> Result<(u32, u32, usize), i32>{
        let bs = usb.block_size as usize;
        if bs == 0{
            return Err(ESP_ERR_INVALID_STATE);
        }
        if bs > MAX_PAYLOAD{
            return Err(ESP_ERR_INVALID_SIZE);
        }
        if(MAX_PAYLOAD % bs) != 0{
            return Err(ESP_ERR_INVALID_STATE);
        }

        let total_bytes = (nblocks_total as usize).checked_mul(bs).ok_or(ESP_ERR_INVALID_SIZE)?;
        let offset_bytes = (chunk_idx as usize).checked_mul(MAX_PAYLOAD).ok_or(ESP_ERR_INVALID_SIZE)?;
        if offset_bytes >= total_bytes{
            return Err(ESP_ERR_INVALID_ARG);
        }

        let chunk_len = core::cmp::min(MAX_PAYLOAD, total_bytes - offset_bytes);
        if (chunk_len % bs) != 0{
            return Err(ESP_ERR_INVALID_SIZE);
        }

        let lba_i = lba_start + (offset_bytes / bs) as u32;
        let nblocks_i = (chunk_len / bs) as u32;

        let bc = usb.block_count;
        if bc == 0{
            return Err(ESP_ERR_INVALID_STATE);
        }
        if lba_i.checked_add(nblocks_i).is_none() || (lba_i + nblocks_i) > bc{
            return Err(ESP_ERR_INVALID_ARG);
        }

        Ok((lba_i, nblocks_i, chunk_len))
    }

    pub fn run(&mut self) -> !{
        assert!(self.inited);

        let mut rx_dma = DmaBuf::new(RX_LEN);
        let mut tx_dma = DmaBuf::new(RX_LEN);

        loop{
            
            if !Self::spi_slave_xfer(&mut rx_dma, &tx_dma, HDR_LEN){
                continue;
            }

            let req = unsafe{
                core::ptr::read_unaligned(rx_dma.as_ptr() as *const Header)
            };

            if !req.is_valid(){
                log::warn!("spi_link: invalid header");
                continue;
            }

            let Some(cmd) = req.cmd_enum() else{
                log::warn!("spi_link: unknown cmd={}", req.cmd);
                self.arm_response(&req, ESP_ERR_INVALID_ARG, &[]);
                self.send_response(&mut rx_dma, &mut tx_dma);
                continue;
            };

            let arg0 = req.arg0;
            let arg1 = req.arg1;
            let chunk_idx = req.reserved;

            //log::info!("spi_link: cmd={:?} seq={} arg0={} arg1={} chunk_idx={}", cmd, seq, arg0, arg1, chunk_idx);

            match cmd {
                Cmd::GetStatus => {
                    // payload: 1 byte status (0/1/2)
                    let payload = if let Some(usb) = get_global_mass_storage(){
                        let status = usb.bd_status();
                        [match status {
                            BlockDevStatus::NotPresent => 0u8,
                            BlockDevStatus::NotReady => 1u8,
                            BlockDevStatus::Ready => 2u8,
                        }]
                    }
                    else{
                        [0u8]
                    };
                    //log::info!("spi_link: REQ GetStatus payload={}", payload[0]);
                    self.arm_response(&req, ESP_OK, &payload);
                    self.send_response(&mut rx_dma, &mut tx_dma);
                }

                Cmd::GetCapacity => {
                    // payload: 8 bytes (block_size, block_count)
                    let Some(usb) = get_global_mass_storage() else{
                        self.arm_response(&req, ESP_ERR_INVALID_STATE, &[]);
                        self.send_response(&mut rx_dma, &mut tx_dma);
                        continue;
                    };

                    match usb.bd_refresh_capacity(){
                        Ok((bs, bc)) => {
                            let capacity = payload::encode_capacity(bs, bc);
                            //log::info!("spi_link: REQ GetCapacity seq={}", seq);
                            self.arm_response_with_arg1(&req, ESP_OK, capacity.len() as u32, &capacity);
                        }
                        Err(e) => {
                            self.arm_response(&req, e, &[]);
                        }
                    }

                    self.send_response(&mut rx_dma, &mut tx_dma);
                }

                Cmd::Read => {
                    let t_total0 = unsafe{esp_timer_get_time() as i64};

                    let Some(usb) = get_global_mass_storage() else{
                        self.arm_response(&req, ESP_ERR_INVALID_STATE, &[]);
                        self.send_response(&mut rx_dma, &mut tx_dma);
                        continue;
                    };

                    Self::ensure_capacity_known(usb);

                    let (lba_i, nblocks_i, chunk_len) = {
                        match Self::compute_chunk(usb, arg0, arg1, chunk_idx){
                            Ok(v) => v,
                            Err(e) => {
                                self.arm_response(&req, e, &[]);
                                self.send_response(&mut rx_dma, &mut tx_dma);
                                continue;
                            }
                        }
                    };

                    let t_usb0 = unsafe{esp_timer_get_time() as i64};
                    let read_res = usb.bd_read_blocks(lba_i, nblocks_i, &mut self.block_buf[..chunk_len]);
                    let t_usb1 = unsafe{esp_timer_get_time() as i64};

                    self.perf.read_ops += 1;
                    self.perf.read_usb_us += (t_usb1 - t_usb0) as u64;
                    self.perf.read_total_us += (t_usb1 - t_total0) as u64;
                    self.perf.log_if_needed();


                    match read_res{
                        Ok(()) => {
                            //log::info!("spi_link: REQ Read seq={} lba_start={} nblocks_total={} chunk_idx={} -> lba_i={} nblocks_i={} chunk_len={}", seq, lba_start, nblocks_total, chunk_idx, lba_i, nblocks_i, chunk_len);
                            self.arm_response_from_block_buf(&req, ESP_OK, chunk_len as u32, chunk_len);
                        }
                        Err(e) => {
                            log::error!("spi_link: Read bd_read_blocks err={} lba={} n={}", e, lba_i, nblocks_i);
                            self.arm_response(&req, e, &[]);
                        }
                    }

                    self.send_response(&mut rx_dma, &mut tx_dma);
                }

                Cmd::Write => {
                    let t_total0 = unsafe{esp_timer_get_time() as i64};

                    let Some(usb) = get_global_mass_storage() else {
                        log::error!("spi_link: Write sans USB");
                        tx_dma.clear();
                        Self::ready_high();
                        Self::spi_slave_xfer(&mut rx_dma, &tx_dma, MAX_PAYLOAD + CRC_LEN);
                        Self::ready_low();
                        self.arm_response(&req, ESP_ERR_INVALID_STATE, &[]);
                        self.send_response(&mut rx_dma, &mut tx_dma);
                        continue;
                    };
                    Self::ensure_capacity_known(usb);
                    
                    let (lba_i, nblocks_i, chunk_len) = 
                        match Self::compute_chunk(usb, arg0, arg1, chunk_idx){
                            Ok(v) => v,
                            Err(e) => {
                                log::error!("spi_link: Write compute_chunk err={}", e);
                                tx_dma.clear();
                                Self::ready_high();
                                Self::spi_slave_xfer(&mut rx_dma, &tx_dma, MAX_PAYLOAD + CRC_LEN);
                                Self::ready_low();
                                self.arm_response(&req, e, &[]);
                                self.send_response(&mut rx_dma, &mut tx_dma);
                                continue;
                            }
                        };

                    tx_dma.clear();
                    Self::ready_high();
                    if !Self::spi_slave_xfer(&mut rx_dma, &tx_dma, chunk_len + CRC_LEN){
                        Self::ready_low();
                        continue;
                    }
                    Self::ready_low();

                    // verify CRC32 on received write payload
                    let rx_data = rx_dma.as_slice();
                    let received_crc = u32::from_le_bytes([
                        rx_data[chunk_len], rx_data[chunk_len + 1],
                        rx_data[chunk_len + 2], rx_data[chunk_len + 3],
                    ]);
                    let computed_crc = spi_crc32(&rx_data[..chunk_len]);
                    if received_crc != computed_crc{
                        log::error!("spi_link: Write CRC mismatch lba={} chunk={} recv=0x{:08x} comp=0x{:08x}", lba_i, chunk_idx, received_crc, computed_crc);
                        self.arm_response(&req, ESP_ERR_INVALID_CRC, &[]);
                        self.send_response(&mut rx_dma, &mut tx_dma);
                        continue;
                    }

                    let t_usb0 = unsafe{esp_timer_get_time() as i64};
                    let write_res = usb.bd_write_blocks(lba_i, nblocks_i, &rx_dma.as_slice()[..chunk_len]);
                    let t_usb1 = unsafe{esp_timer_get_time() as i64};

                    self.perf.write_ops += 1;
                    self.perf.write_usb_us += (t_usb1 - t_usb0) as u64;
                    self.perf.write_total_us += (t_usb1 - t_total0) as u64;
                    self.perf.log_if_needed();


                    match write_res{
                        Ok(()) => {
                            self.arm_response_with_arg1(&req, ESP_OK, chunk_len as u32, &[]);
                        }
                        Err(e) => {
                            log::error!("spi_link: Write failed err={} lba={} n={}", e, lba_i, nblocks_i);
                            self.arm_response(&req, e, &[]);
                        }
                    }

                    self.send_response(&mut rx_dma, &mut tx_dma);
                }


                Cmd::Flush => {
                    //log::info!("spi_link: REQ Flush seq={}", seq);
                    self.arm_response(&req, ESP_OK, &[]);
                    self.send_response(&mut rx_dma, &mut tx_dma);
                }
            }
        }
    }
}
