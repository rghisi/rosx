use crate::debugport::{render_dec, render_hex};
use crate::pio;
use crate::COM1_PORT;

const INTERRUPT_ENABLE_REGISTER: u16 = 1;
const FIFO_CONTROL_REGISTER: u16 = 2;
const LINE_CONTROL_REGISTER: u16 = 3;
const MODEM_CONTROL_REGISTER: u16 = 4;
const LINE_STATUS_REGISTER: u16 = 5;

const DIVISOR_LATCH_BAUD_FACTOR_1: u8 = 1;
const DIVISOR_LATCH_BAUD_FACTOR_HIGH: u8 = 0;
const LINE_CONTROL_8N1: u8 = 0x03;
const LINE_CONTROL_ACCESS_BAUD_DIVISOR: u8 = 0x80;
const MODEM_CONTROL_DATA_TERMINAL_READY: u8 = 0x01;
const MODEM_CONTROL_REQUEST_TO_SEND: u8 = 0x02;
const MODEM_CONTROL_OUT2: u8 = 0x08;
const FIFO_CONTROL_ENABLE_WITH_14_BYTE_TRIGGER: u8 = 0xC7;
const LINE_STATUS_TRANSMITTER_HOLDING_EMPTY: u8 = 0x20;

pub struct Serial {
    port: u16,
}

impl Serial {
    pub const COM1: Serial = Serial { port: COM1_PORT };

    pub fn init(&self) {
        pio::outb(self.port + INTERRUPT_ENABLE_REGISTER, 0x00);
        pio::outb(self.port + LINE_CONTROL_REGISTER, LINE_CONTROL_ACCESS_BAUD_DIVISOR);
        pio::outb(self.port, DIVISOR_LATCH_BAUD_FACTOR_1);
        pio::outb(self.port + INTERRUPT_ENABLE_REGISTER, DIVISOR_LATCH_BAUD_FACTOR_HIGH);
        pio::outb(self.port + LINE_CONTROL_REGISTER, LINE_CONTROL_8N1);
        pio::outb(
            self.port + MODEM_CONTROL_REGISTER,
            MODEM_CONTROL_DATA_TERMINAL_READY
                | MODEM_CONTROL_REQUEST_TO_SEND
                | MODEM_CONTROL_OUT2,
        );
        pio::outb(self.port + FIFO_CONTROL_REGISTER, FIFO_CONTROL_ENABLE_WITH_14_BYTE_TRIGGER);
    }

    pub fn write_byte(&self, byte: u8) {
        while pio::inb(self.port + LINE_STATUS_REGISTER) & LINE_STATUS_TRANSMITTER_HOLDING_EMPTY
            == 0
        {
        }
        pio::outb(self.port, byte);
    }

    pub fn write_str(&self, text: &str) {
        for byte in text.bytes() {
            self.write_byte(byte);
        }
    }

    pub fn write_line(&self, text: &str) {
        self.write_str(text);
        self.write_crlf();
    }

    pub fn write_crlf(&self) {
        self.write_str("\r\n");
    }

    pub fn write_hex(&self, value: u64, nibbles: u32) {
        render_hex(|byte| self.write_byte(byte), value, nibbles);
    }

    pub fn write_dec(&self, value: u64) {
        render_dec(|byte| self.write_byte(byte), value);
    }
}

impl core::fmt::Write for Serial {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        for byte in text.bytes() {
            self.write_byte(byte);
        }
        Ok(())
    }
}
