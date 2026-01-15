use core::ptr;
use esp_idf_sys::*;

use super::pins::*;
use super::protocol::*;
use crate::usb_mass_storage::UsbMassStorage;
use crate::usb_mass_storage::block_device::*;
use crate::usb_mass_storage::get_global_mass_storage;

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
const RX_LEN: usize = HDR_LEN + MAX_PAYLOAD;
pub struct SpiLink{
    inited: bool,
    //2-phase storage
    resp_ready: bool,
    resp_len: usize,
    resp_buf: [u8; core::mem::size_of::<Header>() + MAX_PAYLOAD],

    block_buf: [u8; MAX_PAYLOAD]
}

impl SpiLink {
    pub fn new() -> Self{
        Self{
            inited: false,
            resp_ready: false,
            resp_len: 0,
            resp_buf: [0u8; RX_LEN],
            block_buf: [0u8; MAX_PAYLOAD],
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

        self.resp_ready = true;
        log::info!("spi_link: armed response len={}, arg1={}", self.resp_len, arg1);

        unsafe{
            gpio_set_level(PIN_READY as gpio_num_t, 1);
        }

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

        // payload from self.block_buf without surviving borrow
        let pay_len = core::cmp::min(len, MAX_PAYLOAD);
        if pay_len > 0 {
            self.resp_buf[HDR_LEN..HDR_LEN + pay_len].copy_from_slice(&self.block_buf[..pay_len]);
            self.resp_len = HDR_LEN + pay_len;
        }
        else{
            self.resp_len = HDR_LEN;
        }

        self.resp_ready = true;
        log::info!("spi_link: armed response len={}, arg1={}", self.resp_len, arg1);
        unsafe{ 
            gpio_set_level(PIN_READY as gpio_num_t, 1);
        }
    }


    #[inline]
    fn arm_response(&mut self, req: &Header, status: i32, payload: &[u8]){
        self.arm_response_with_arg1(req, status, 0, payload);
    }

    fn disarm_response(&mut self){
        self.resp_ready = false;
        self.resp_len = 0;
        unsafe{
            gpio_set_level(PIN_READY as gpio_num_t, 0);
        }
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

    pub fn run(&mut self) -> ! {
        assert!(self.inited);

        let mut rx_dma = DmaBuf::new(RX_LEN);
        let mut tx_dma = DmaBuf::new(RX_LEN);

        loop{
            tx_dma.clear();

            if self.resp_ready{
                let n = core::cmp::min(self.resp_len, RX_LEN);
                tx_dma.as_mut_slice()[..n].copy_from_slice(&self.resp_buf[..n]);
            }
            else{

            }

            let mut t: spi_slave_transaction_t = unsafe { core::mem::zeroed() };
            t.length = RX_LEN * 8;
            t.rx_buffer = rx_dma.as_mut_ptr() as *mut _;
            t.tx_buffer = tx_dma.as_ptr() as *const _;
            t.user = ptr::null_mut();

            unsafe{
                let err = spi_slave_transmit(
                    spi_host_device_t_SPI3_HOST,
                    &mut t,
                    TickType_t::MAX,
                );

                if err != ESP_OK {
                    log::error!("spi_slave_transmit err={}", err);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
            }

            if self.resp_ready{
                self.disarm_response();
                continue;
            }

            let rx = rx_dma.as_slice();
            let req = unsafe{
                core::ptr::read_unaligned(rx.as_ptr() as *const Header)
            };

            if !req.is_valid(){
                log::warn!("spi_link: invalid header");
                continue;
            }

            let Some(cmd) = req.cmd_enum() else{
                log::warn!("spi_link: unknown command {}", req.cmd);
                self.arm_response(&req, ESP_ERR_INVALID_ARG, &[]);
                continue;
            };

            let seq = req.seq;
            let arg0 = req.arg0;
            let arg1 = req.arg1;
            let chunk_idx = req.reserved;
            log::info!("spi_link: cmd={:?} seq={} arg0={} arg1={} chunk_idx={}", cmd, seq, arg0, arg1, chunk_idx);

            match cmd {
                Cmd::GetStatus => {
                    // payload: 1 byte status (0/1/2)
                    let payload = if let Some(usb) = get_global_mass_storage(){
                        let status = usb.bd_status();
                        [match status {
                            BlockDevStatus::NotPresent => 0,
                            BlockDevStatus::NotReady => 1,
                            BlockDevStatus::Ready => 2,
                        }]
                    }
                    else{
                        [0u8]
                    };
                    log::info!("spi_link: REQ GetStatus seq={}", seq);
                    self.arm_response(&req, ESP_OK, &payload);
                }

                Cmd::GetCapacity => {
                    // payload: 8 bytes (block_size, block_count)
                    let Some(usb) = get_global_mass_storage() else{
                        self.arm_response(&req, ESP_ERR_INVALID_STATE, &[]);
                        continue;
                    };

                    match usb.bd_refresh_capacity(){
                        Ok((bs, bc)) => {
                            let capacity = payload::encode_capacity(bs, bc);
                            log::info!("spi_link: REQ GetCapacity seq={}", seq);
                            self.arm_response_with_arg1(&req, ESP_OK, capacity.len() as u32, &capacity);
                        }
                        Err(e) => {
                            self.arm_response(&req, e, &[]);
                        }
                    }
                }

                Cmd::Read => {
                    let lba_start = arg0;
                    let nblocks_total = arg1;
                    let chunk_idx = req.reserved;

                    let Some(usb) = get_global_mass_storage() else{
                        self.arm_response(&req, ESP_ERR_INVALID_STATE, &[]);
                        continue;
                    };

                    Self::ensure_capacity_known(usb);

                    let (lba_i, nblocks_i, chunk_len) = {
                        match Self::compute_chunk(usb, lba_start, nblocks_total, chunk_idx){
                            Ok(v) => v,
                            Err(e) => {
                                self.arm_response(&req, e, &[]);
                                continue;
                            }
                        }
                    };

                    let read_res = {
                        let buf = &mut self.block_buf[..chunk_len];
                        usb.bd_read_blocks(lba_i, nblocks_i, buf)
                    };

                    match read_res{
                        Ok(()) => {
                            log::info!("spi_link: REQ Read seq={} lba_start={} nblocks_total={} chunk_idx={} -> lba_i={} nblocks_i={} chunk_len={}", seq, lba_start, nblocks_total, chunk_idx, lba_i, nblocks_i, chunk_len);
                            self.arm_response_from_block_buf(&req, ESP_OK, chunk_len as u32, chunk_len);
                        }
                        Err(e) => {
                            self.arm_response(&req, e, &[]);
                        }
                    }
                }

                Cmd::Write => {
                    let lba = arg0;
                    let nblocks_total = arg1;

                    let Some(usb) = get_global_mass_storage() else {
                    self.arm_response(&req, ESP_ERR_INVALID_STATE, &[]);
                        continue;
                    };

                    // test: only 1 block for now
                    if nblocks_total != 1 {
                        self.arm_response(&req, ESP_ERR_NOT_SUPPORTED, &[]);
                            continue;
                    }
                    if chunk_idx != 0 {
                        self.arm_response(&req, ESP_ERR_INVALID_ARG, &[]);
                        continue;
                    }

                    Self::ensure_capacity_known(usb);
                    let bs = usb.block_size as usize;
                    if bs == 0 {
                        self.arm_response(&req, ESP_ERR_INVALID_STATE, &[]);
                        continue;
                    }
                    if bs > MAX_PAYLOAD {
                        self.arm_response(&req, ESP_ERR_INVALID_SIZE, &[]);
                        continue;
                    }

                    // payload is right after header
                    let payload_off = HDR_LEN;
                    let payload_end = payload_off + bs;
                    if payload_end > RX_LEN {
                        self.arm_response(&req, ESP_ERR_INVALID_SIZE, &[]);
                        continue;
                    }

                    let data = &rx[payload_off..payload_end];

                    match usb.bd_write_blocks(lba, 1, data) {
                        Ok(()) => {
                            log::info!("spi_link: REQ Write seq={} lba={} nblocks={}", seq, lba, nblocks_total);
                            // arg1 = bytes written (useful now; required later for chunking)
                            self.arm_response_with_arg1(&req, ESP_OK, bs as u32, &[]);
                        }
                        Err(e) => {
                            log::error!(
                                "spi_link: Write failed err={} seq={} lba={} nblocks={}",
                                e,
                                seq,
                                lba,
                                nblocks_total
                            );
                            self.arm_response(&req, e, &[]);
                        }
                    }
                }


                Cmd::Flush => {
                    log::info!("spi_link: REQ Flush seq={}", seq);
                    self.arm_response(&req, ESP_OK, &[]);
                }
            }
        }
    }
}
