use kernel::platform::interrupt::{InterruptController, InterruptError, Polarity, Result, TriggerMode};
use pic8259::ChainedPics;
use spin::Mutex;
use x86_64::instructions::port::Port;

pub const PIC_1_OFFSET: u8 = 0x20;
pub const PIC_2_OFFSET: u8 = 0x28;

const PIC1_IMR_PORT: u16 = 0x21;
const PIC2_IMR_PORT: u16 = 0xA1;

pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

pub struct PicController;

impl InterruptController for PicController {
    const MAX_IRQ: u32 = 15;
    const VECTOR_BASE: u8 = PIC_1_OFFSET;

    fn new() -> Self {
        unsafe {
            PICS.lock().initialize();
            Port::<u8>::new(PIC1_IMR_PORT).write(0xFF);
            Port::<u8>::new(PIC2_IMR_PORT).write(0xFF);
        }
        PicController
    }

    fn configure(
        &mut self,
        irq: u32,
        _vector: u8,
        _trigger: TriggerMode,
        _polarity: Polarity,
    ) -> Result {
        if irq > Self::MAX_IRQ {
            return Err(InterruptError::InvalidIrq);
        }
        Ok(())
    }

    fn mask(&mut self, irq: u32) -> Result {
        update_imr_bit(irq, true)
    }

    fn unmask(&mut self, irq: u32) -> Result {
        update_imr_bit(irq, false)
    }

    fn eoi(&mut self, irq: u32) {
        let vector = Self::VECTOR_BASE + irq as u8;
        unsafe {
            PICS.lock().notify_end_of_interrupt(vector);
        }
    }
}

fn update_imr_bit(irq: u32, masked: bool) -> Result {
    if irq > 15 {
        return Err(InterruptError::InvalidIrq);
    }
    unsafe {
        if irq < 8 {
            let mut imr = Port::<u8>::new(PIC1_IMR_PORT);
            let current = imr.read();
            let bit = 1u8 << irq;
            imr.write(if masked { current | bit } else { current & !bit });
        } else {
            let mut imr = Port::<u8>::new(PIC2_IMR_PORT);
            let current = imr.read();
            let bit = 1u8 << (irq - 8);
            imr.write(if masked { current | bit } else { current & !bit });
        }
    }
    Ok(())
}
