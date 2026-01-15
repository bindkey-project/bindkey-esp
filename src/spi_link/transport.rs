use core::ptr;
use esp_idf_sys::*;

use super::pins::*;
use super::protocol::*;

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

pub struct SpiLink{
    inited: bool,
    //2-phase storage
    resp_ready: bool,
    resp_len: usize,
    resp_buf: [u8; core::mem::size_of::<Header>() + MAX_PAYLOAD]
}

impl SpiLink {
    pub fn new() -> Self{
        Self{
            inited: false,
            resp_ready: false,
            resp_len: 0,
            resp_buf: [0u8; core::mem::size_of::<Header>() + MAX_PAYLOAD],
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
            buscfg.max_transfer_sz = (core::mem::size_of::<Header>() + MAX_PAYLOAD) as i32; // 528 bytes


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

    fn arm_response(&mut self, req: &Header, status: i32, payload: &[u8]){
        let resp_hdr = Header::response_for(req, status);

        let hdr_bytes = unsafe{
            core::slice::from_raw_parts(
                (&resp_hdr as *const Header) as *const u8,
                core::mem::size_of::<Header>(),
            )
        };

        self.resp_buf[..core::mem::size_of::<Header>()].copy_from_slice(hdr_bytes);

        let max_pay = self.resp_buf.len() - core::mem::size_of::<Header>();
        let pay_len = core::cmp::min(payload.len(), max_pay);
        if pay_len > 0{
            let off = core::mem::size_of::<Header>();
            self.resp_buf[off..off + pay_len].copy_from_slice(&payload[..pay_len]);
            self.resp_len = off + pay_len;
        }
        else{
            self.resp_len = core::mem::size_of::<Header>();
        }

        self.resp_ready = true;
        log::info!("spi_link: armed response len={}", self.resp_len);

        unsafe{
            gpio_set_level(PIN_READY as gpio_num_t, 1);
        }

    }

    fn disarm_response(&mut self){
        self.resp_ready = false;
        self.resp_len = 0;
        unsafe{
            gpio_set_level(PIN_READY as gpio_num_t, 0);
        }
    }

    pub fn run(&mut self) -> ! {
        assert!(self.inited);

        const RX_LEN: usize = core::mem::size_of::<Header>() + MAX_PAYLOAD;

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
                log::info!(
                    "RX[0..4] = {:02X} {:02X} {:02X} {:02X}",
                    rx[0], rx[1], rx[2], rx[3]
                );

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
            log::info!("spi_link: cmd={:?} seq={} arg0={} arg1={}", cmd, seq, arg0, arg1);

            match cmd {
                Cmd::GetStatus => {
                    // payload: 1 byte status (0/1/2)
                    // dummy: Ready
                    let payload = [2u8];
                    log::info!("spi_link: REQ GetStatus seq={}", seq);
                    self.arm_response(&req, ESP_OK, &payload);
                }

                Cmd::GetCapacity => {
                    // payload: 8 bytes (block_size, block_count)
                    // dummy values for now
                    let cap = payload::encode_capacity(512, 123456);
                    log::info!("spi_link: REQ GetCapacity seq={}", seq);
                    self.arm_response(&req, ESP_OK, &cap);
                }

                Cmd::Read => {
                    // later: read blocks from UsbMassStorage
                    log::info!("spi_link: REQ Read seq={} lba={} nblocks={}", seq, arg0, arg1);
                    self.arm_response(&req, ESP_ERR_NOT_SUPPORTED, &[]);
                }

                Cmd::Write => {
                    // later: write blocks to UsbMassStorage
                    log::info!("spi_link: REQ Write seq={} lba={} nblocks={}", seq, arg0, arg1);
                    self.arm_response(&req, ESP_ERR_NOT_SUPPORTED, &[]);
                }

                Cmd::Flush => {
                    log::info!("spi_link: REQ Flush seq={}", seq);
                    self.arm_response(&req, ESP_OK, &[]);
                }
            }
        }
    }
}
