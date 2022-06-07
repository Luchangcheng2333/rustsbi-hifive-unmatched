use core::convert::Infallible;
use core::ptr::{read_volatile, write_volatile};
use embedded_hal::serial::{Read, Write};

// UART that is initialized by prior steps of bootloading
#[derive(Clone, Copy)]
pub struct Uart;

// UART外设是可以跨上下文共享的
unsafe impl Send for Uart {}
unsafe impl Sync for Uart {}

impl Uart {
    #[inline]
    pub unsafe fn preloaded_uart0() -> Self {
        let divisor = (UART_CLK / UART_BUADRATE_32MCLK_115200) >> 4;
        let lcr_cache: u8 = serial_in(REG_LCR) as u8;
        serial_out(REG_LCR, LCR_DLAB | lcr_cache as u32);
	    serial_out(REG_BRDL,(divisor & 0xff) as u32);
	    serial_out(REG_BRDH, ((divisor >> 8) & 0xff) as u32);
	

	    /* restore the DLAB to access the baud rate divisor registers */
	    serial_out(REG_LCR, lcr_cache as u32);

	    /* 8 data bits, 1 stop bit, no parity, clear DLAB */
	    serial_out(REG_LCR, LCR_CS8 | LCR_1_STB | LCR_PDIS);
	
	    serial_out(REG_MDC, 0); /*disable flow control*/
	
	    /*
	        * Program FIFO: enabled, mode 0 (set for compatibility with quark),
	        * generate the interrupt at 8th byte
	        * Clear TX and RX FIFO
	    */
	    serial_out(REG_FCR, FCR_FIFO | FCR_MODE1 | /*FCR_FIFO_1*/FCR_FIFO_8 | FCR_RCVRCLR | FCR_XMITCLR);
	
	    serial_out(REG_IER, 0);//dis the ser interrupt
        Self {}
    }
}

// Ref: JH7100-secondBoot

impl Read<u8> for Uart {
    type Error = Infallible;

    #[inline]
    fn read(&mut self) -> nb::Result<u8, Self::Error> {
        if serial_in(REG_LSR) & (1 << 0) != 0 {
            Ok(serial_in(REG_RDR) as u8)
        } else {
            Err(nb::Error::WouldBlock)
        }
    }
}

impl Write<u8> for Uart {
    type Error = Infallible;

    #[inline]
    fn write(&mut self, byte: u8) -> nb::Result<(), Infallible> {
        serial_out(REG_THR, byte as u32);
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> nb::Result<(), Infallible> {
        if (serial_in(REG_LSR) & LSR_THRE) != 0 {
            Ok(())
        } else {
            Err(nb::Error::WouldBlock)
        }
    }
}

fn serial_in(offset: u32) -> u32 {
    let offset = offset << 2 as u32;
    unsafe {
        read_volatile((UART_BASE + offset as usize) as *const u32)
    }
}

fn serial_out(offset: u32, val: u32) {
    let offset = offset << 2 as u32;
    unsafe {
        write_volatile((UART_BASE + offset as usize) as *mut u32, val);
    }
}

const UART_BASE: usize = 0x1244_0000;
const REG_THR: u32 = 0x00; /* Transmitter holding reg. */
const REG_RDR: u32 = 0x00; /* Receiver data reg.       */
const REG_LSR: u32 = 0x05; /* Line status reg.         */
const REG_LCR: u32 = 0x03; /* Line control reg.        */
const LCR_DLAB: u32 = 0x80; /* divisor latch access enable */
const REG_BRDL: u32 = 0x00; /* Baud rate divisor (LSB)  */
const REG_BRDH: u32 = 0x01; /* Baud rate divisor (MSB)  */
const LCR_CS8: u32 = 0x03; /* 8 bits data size */
const LCR_1_STB: u32 = 0x01; /* 1 stop bit */
const LCR_PDIS: u32 = 0x00; /* parity disable */
const REG_MDC: u32 = 0x04; /* Modem control reg.       */
const REG_FCR: u32 = 0x02; /* FIFO control reg.        */
const REG_IER: u32 = 0x01; /* Interrupt enable reg.    */
const LSR_THRE: u32 = 0x20; /* transmit holding register empty */
const FCR_FIFO: u32 = 0x01; /* enable XMIT and RCVR FIFO */
const FCR_RCVRCLR: u32 = 0x02; /* clear RCVR FIFO */
const FCR_XMITCLR: u32 = 0x03; /* clear XMIT FIFO */
const FCR_MODE1: u32 = 0x08; /* set receiver in mode 1 */
const FCR_FIFO_8: u32 = 0x80; /* 8 bytes in RCVR FIFO */
const UART_CLK: usize = 100000000;
const UART_BUADRATE_32MCLK_115200: usize = 115200;
