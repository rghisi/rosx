use rosx_boot_common::bios::Regs16;
use rosx_boot_common::debugport::DebugConsole;
use rosx_boot_common::debugport::render_dec;
use rosx_boot_common::debugport::render_hex;

use crate::thunk::BiosThunk;

const INT10: u8 = 0x10;
const TELETYPE_FUNCTION: u16 = 0x0E00;

pub struct BroadcastConsole<'a> {
    debug: DebugConsole,
    thunk: &'a BiosThunk,
}

impl<'a> BroadcastConsole<'a> {
    pub fn new(thunk: &'a BiosThunk) -> BroadcastConsole<'a> {
        BroadcastConsole {
            debug: DebugConsole,
            thunk,
        }
    }

    pub fn write_str(&self, text: &str) {
        self.debug.write_str(text);
        for byte in text.bytes() {
            self.teletype(byte);
        }
    }

    pub fn write_line(&self, text: &str) {
        self.write_str(text);
        self.write_str("\r\n");
    }

    pub fn write_hex(&self, value: u64, nibbles: u32) {
        self.debug.write_hex(value, nibbles);
        render_hex(|byte| self.teletype(byte), value, nibbles);
    }

    pub fn write_dec(&self, value: u64) {
        self.debug.write_dec(value);
        render_dec(|byte| self.teletype(byte), value);
    }

    fn teletype(&self, byte: u8) {
        let mut regs = Regs16::new(TELETYPE_FUNCTION | u16::from(byte), 0, 0, 0);
        self.thunk.call(INT10, &mut regs);
    }
}
