use alloc::vec;
use alloc::vec::Vec;
use core::cmp::min;
use core::fmt::{Display, Formatter};
use usrlib::error::PortError;

pub const ETHERNET_MIN_BYTES: usize = 60;
pub const ETHERNET_MAX_BYTES: usize = 1514;
pub const MAC_BYTES: usize = 6;
pub const TX_BUFFER_FIRST_PAGE: u8 = 0x40;
pub const RX_RING_FIRST_PAGE: u8 = 0x46;
pub const RX_RING_LAST_PAGE: u8 = 0x7F;
pub const RX_RING_STOP_PAGE: u8 = 0x80;
pub const ISA_DEFAULT_BASE: u16 = 0x300;
pub const ISA_DEFAULT_IRQ: u32 = 9;

const TX_BUFFER_ADDRESS: u16 = 0x4000;
const RX_RING_START_ADDRESS: u16 = 0x4600;
const SRAM_TOP_ADDRESS: u16 = 0x8000;
const PROM_ADDRESS: u16 = 0x0000;
const PROM_WORDS: usize = 8;

const RX_HEADER_BYTES: usize = 4;
const RX_HEADER_STATUS_RXOK: u8 = 0x01;
const RX_HEADER_STATUS_ERROR_MASK: u8 = 0x1C;
const RX_HEADER_MIN_TOTAL: u16 = (ETHERNET_MIN_BYTES + RX_HEADER_BYTES) as u16;
const RX_HEADER_MAX_TOTAL: u16 = (ETHERNET_MAX_BYTES + RX_HEADER_BYTES) as u16;

const PORT_COMMAND: u16 = 0x00;
const PORT_PAGE_LOW: u16 = 0x01;
const PORT_PAGE_HIGH: u16 = 0x02;
const PORT_BOUNDARY: u16 = 0x03;
const PORT_TRANSMIT_STATUS: u16 = 0x04;
const PORT_TRANSMIT_COUNT0: u16 = 0x05;
const PORT_TRANSMIT_COUNT1: u16 = 0x06;
const PORT_INTERRUPT_STATUS: u16 = 0x07;
const PORT_REMOTE_START0: u16 = 0x08;
const PORT_REMOTE_START1: u16 = 0x09;
const PORT_REMOTE_COUNT0: u16 = 0x0A;
const PORT_REMOTE_COUNT1: u16 = 0x0B;
const PORT_RECEIVE_CONFIG: u16 = 0x0C;
const PORT_TRANSMIT_CONFIG: u16 = 0x0D;
const PORT_DATA_CONFIG: u16 = 0x0E;
const PORT_INTERRUPT_MASK: u16 = 0x0F;
const PORT_DATA_FIFO: u16 = 0x10;
const PORT_NODE_ADDRESS_FIRST: u16 = 0x01;

const CR_HALT_PAGE_0: u8 = 0x21;
const CR_RUN_PAGE_0: u8 = 0x22;
const CR_HALT_PAGE_1: u8 = 0x61;
const CR_RUN_PAGE_1: u8 = 0x62;
const CR_REMOTE_READ: u8 = 0x0A;
const CR_REMOTE_WRITE: u8 = 0x12;
const CR_TRANSMIT: u8 = 0x26;
const CR_TRANSMIT_COMMAND_BIT: u8 = 0x04;

const ISR_TX_COMPLETE: u8 = 0x02;
const ISR_TX_ERROR: u8 = 0x08;
const ISR_REMOTE_DMA_COMPLETE: u8 = 0x40;
const ISR_RESET: u8 = 0x80;
const ISR_CLEAR_ALL: u8 = 0xFF;
const IMR_ENABLE_EVENTS: u8 = 0x3F;

const DCR_16_BIT_LONG_TIMING: u8 = 0x49;
const RCR_PHYSICAL_MULTICAST_BROADCAST: u8 = 0x70;
const TCR_INTERNAL_LOOPBACK: u8 = 0x02;
const TCR_NORMAL: u8 = 0x00;
const TSR_TRANSMIT_ERROR_MASK: u8 = 0xF8;

const RESET_POLL_ATTEMPTS: usize = 50;
const DMA_POLL_ATTEMPTS: usize = 200;
const POLL_SLEEP_MS: u64 = 1;

pub trait NicBus {
    fn read_byte(&mut self, port: u16) -> Result<u8, PortError>;
    fn write_byte(&mut self, port: u16, value: u8) -> Result<(), PortError>;
    fn read_words(&mut self, port: u16, destination: &mut [u16]) -> Result<(), PortError>;
    fn write_words(&mut self, port: u16, source: &[u16]) -> Result<(), PortError>;
    fn sleep_ms(&mut self, millis: u64);
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NicFault {
    Absent,
    RemoteDmaTimeout,
    TransmitTimeout,
    TransmitBusy,
    TransmitFailed,
    ParMismatch,
    Oversize,
    Denied(PortError),
}

impl Display for NicFault {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            NicFault::Absent => write!(f, "NIC absent, reset never completed"),
            NicFault::RemoteDmaTimeout => write!(f, "remote DMA timed out"),
            NicFault::TransmitTimeout => write!(f, "transmit timed out"),
            NicFault::TransmitBusy => write!(f, "transmit already in flight"),
            NicFault::TransmitFailed => write!(f, "NIC refused the transmission"),
            NicFault::ParMismatch => write!(f, "node address rejected by the NIC"),
            NicFault::Oversize => write!(f, "frame size rejected"),
            NicFault::Denied(error) => Display::fmt(error, f),
        }
    }
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct NicCounters {
    pub transmitted_frames: u32,
    pub transmit_errors: u32,
    pub received_frames: u32,
    pub receive_errors: u32,
    pub receive_overruns: u32,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct RxHeader {
    status: u8,
    next_page: u8,
    stored_length: u16,
}

impl RxHeader {
    fn payload_length(&self) -> Option<usize> {
        if self.status & RX_HEADER_STATUS_RXOK == 0 {
            return None;
        }
        if self.status & RX_HEADER_STATUS_ERROR_MASK != 0 {
            return None;
        }
        if self.stored_length < RX_HEADER_MIN_TOTAL || self.stored_length > RX_HEADER_MAX_TOTAL {
            return None;
        }
        Some(self.stored_length as usize - RX_HEADER_BYTES)
    }
}

fn within_ring(page: u8) -> bool {
    (RX_RING_FIRST_PAGE..=RX_RING_LAST_PAGE).contains(&page)
}

fn boundary_before(page: u8) -> u8 {
    if page == RX_RING_FIRST_PAGE {
        RX_RING_LAST_PAGE
    } else {
        page - 1
    }
}

fn frame_to_padded_words(frame: &[u8]) -> Vec<u16> {
    let word_count = frame.len().div_ceil(2);
    let mut words = vec![0u16; word_count];
    for (index, pair) in frame.chunks_exact(2).enumerate() {
        words[index] = u16::from_le_bytes([pair[0], pair[1]]);
    }
    if let Some(&tail) = frame.last().filter(|_| frame.len() % 2 == 1) {
        words[word_count - 1] = tail as u16;
    }
    words
}

struct RemoteReadPlan {
    start_address: u16,
    direct_bytes: usize,
    wrapped_bytes: usize,
}

fn plan_remote_read(start_address: u16, transfer_bytes: usize) -> RemoteReadPlan {
    let direct_bytes = min(transfer_bytes, (SRAM_TOP_ADDRESS - start_address) as usize);
    RemoteReadPlan {
        start_address,
        direct_bytes,
        wrapped_bytes: transfer_bytes - direct_bytes,
    }
}

pub struct Ne2k<B: NicBus> {
    bus: B,
    base: u16,
    rx_page: u8,
    mac: [u8; MAC_BYTES],
    counters: NicCounters,
}

impl<B: NicBus> Ne2k<B> {
    pub fn new(base: u16, bus: B) -> Ne2k<B> {
        Ne2k {
            bus,
            base,
            rx_page: RX_RING_FIRST_PAGE,
            mac: [0u8; MAC_BYTES],
            counters: NicCounters::default(),
        }
    }

    pub fn base(&self) -> u16 {
        self.base
    }

    pub fn mac(&self) -> [u8; MAC_BYTES] {
        self.mac
    }

    pub fn counters(&self) -> NicCounters {
        self.counters
    }

    pub fn initialize(&mut self) -> Result<[u8; MAC_BYTES], NicFault> {
        self.halt_and_wait_reset()?;
        self.out(PORT_DATA_CONFIG, DCR_16_BIT_LONG_TIMING)?;
        self.out(PORT_PAGE_LOW, RX_RING_FIRST_PAGE)?;
        self.out(PORT_PAGE_HIGH, RX_RING_STOP_PAGE)?;
        self.out(PORT_BOUNDARY, RX_RING_FIRST_PAGE)?;
        self.out(PORT_COMMAND, CR_HALT_PAGE_1)?;
        self.out(PORT_INTERRUPT_STATUS, RX_RING_FIRST_PAGE + 1)?;
        self.out(PORT_COMMAND, CR_HALT_PAGE_0)?;
        self.rx_page = RX_RING_FIRST_PAGE + 1;
        self.mac = self.read_persisted_mac()?;
        self.program_node_address()?;
        self.out(PORT_RECEIVE_CONFIG, RCR_PHYSICAL_MULTICAST_BROADCAST)?;
        self.out(PORT_TRANSMIT_CONFIG, TCR_INTERNAL_LOOPBACK)?;
        self.out(PORT_INTERRUPT_STATUS, ISR_CLEAR_ALL)?;
        self.out(PORT_INTERRUPT_MASK, IMR_ENABLE_EVENTS)?;
        self.out(PORT_COMMAND, CR_RUN_PAGE_0)?;
        self.out(PORT_TRANSMIT_CONFIG, TCR_NORMAL)?;
        Ok(self.mac)
    }

    pub fn clear_interrupt_flags(&mut self) -> Result<u8, NicFault> {
        let status = self.in_byte(PORT_INTERRUPT_STATUS)?;
        self.out(PORT_INTERRUPT_STATUS, status)?;
        Ok(status)
    }

    pub fn recover_overrun(&mut self) -> Result<(), NicFault> {
        self.out(PORT_BOUNDARY, RX_RING_FIRST_PAGE)?;
        self.out(PORT_COMMAND, CR_RUN_PAGE_1)?;
        self.out(PORT_INTERRUPT_STATUS, RX_RING_FIRST_PAGE + 1)?;
        self.out(PORT_COMMAND, CR_RUN_PAGE_0)?;
        self.rx_page = RX_RING_FIRST_PAGE + 1;
        self.counters.receive_overruns += 1;
        Ok(())
    }

    pub fn transmit(&mut self, frame: &[u8]) -> Result<(), NicFault> {
        if frame.is_empty() || frame.len() > ETHERNET_MAX_BYTES {
            return Err(NicFault::Oversize);
        }
        if self.in_byte(PORT_COMMAND)? & CR_TRANSMIT_COMMAND_BIT != 0 {
            return Err(NicFault::TransmitBusy);
        }
        let words = frame_to_padded_words(frame);
        self.remote_write(TX_BUFFER_ADDRESS, &words)?;
        self.out(PORT_COMMAND, CR_RUN_PAGE_0)?;
        self.out(PORT_TRANSMIT_STATUS, TX_BUFFER_FIRST_PAGE)?;
        self.out(PORT_TRANSMIT_COUNT0, frame.len() as u8)?;
        self.out(PORT_TRANSMIT_COUNT1, (frame.len() >> 8) as u8)?;
        self.out(PORT_COMMAND, CR_TRANSMIT)?;
        let status = self.poll_isr(
            ISR_TX_COMPLETE | ISR_TX_ERROR,
            DMA_POLL_ATTEMPTS,
            NicFault::TransmitTimeout,
        )?;
        let transmit_status = self.in_byte(PORT_TRANSMIT_STATUS)?;
        self.out(PORT_INTERRUPT_STATUS, status)?;
        if status & ISR_TX_ERROR != 0 || transmit_status & TSR_TRANSMIT_ERROR_MASK != 0 {
            self.counters.transmit_errors += 1;
            return Err(NicFault::TransmitFailed);
        }
        self.counters.transmitted_frames += 1;
        Ok(())
    }

    pub fn receive(&mut self) -> Result<Option<Vec<u8>>, NicFault> {
        loop {
            let hardware_page = self.current_page()?;
            if self.rx_page == hardware_page {
                return Ok(None);
            }
            let page = self.rx_page;
            let header = self.read_rx_header(page)?;
            if !within_ring(header.next_page) {
                self.counters.receive_errors += 1;
                self.rx_page = hardware_page;
                continue;
            }
            self.rx_page = header.next_page;
            self.out(PORT_BOUNDARY, boundary_before(header.next_page))?;
            let Some(payload_length) = header.payload_length() else {
                self.counters.receive_errors += 1;
                continue;
            };
            let frame = self.read_rx_payload(page, payload_length)?;
            self.counters.received_frames += 1;
            return Ok(Some(frame));
        }
    }

    fn halt_and_wait_reset(&mut self) -> Result<(), NicFault> {
        self.out(PORT_COMMAND, CR_HALT_PAGE_0)?;
        for _ in 0..RESET_POLL_ATTEMPTS {
            if self.in_byte(PORT_INTERRUPT_STATUS)? & ISR_RESET != 0 {
                self.out(PORT_INTERRUPT_STATUS, ISR_RESET)?;
                return Ok(());
            }
            self.bus.sleep_ms(POLL_SLEEP_MS);
        }
        Err(NicFault::Absent)
    }

    fn current_page(&mut self) -> Result<u8, NicFault> {
        self.out(PORT_COMMAND, CR_RUN_PAGE_1)?;
        let page = self.in_byte(PORT_INTERRUPT_STATUS)?;
        self.out(PORT_COMMAND, CR_RUN_PAGE_0)?;
        Ok(page)
    }

    fn read_persisted_mac(&mut self) -> Result<[u8; MAC_BYTES], NicFault> {
        let mut words = [0u16; PROM_WORDS];
        self.remote_read(PROM_ADDRESS, &mut words)?;
        let mac: [u8; MAC_BYTES] = words[..MAC_BYTES]
            .iter()
            .map(|word| *word as u8)
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap();
        if mac.iter().all(|byte| *byte == 0 || *byte == 0xFF) {
            return Ok(self.local_administered_mac());
        }
        Ok(mac)
    }

    fn local_administered_mac(&self) -> [u8; MAC_BYTES] {
        [0x02, 0x52, 0x4E, 0x45, 0x32, (self.base ^ (self.base >> 8)) as u8]
    }

    fn program_node_address(&mut self) -> Result<(), NicFault> {
        self.out(PORT_COMMAND, CR_HALT_PAGE_1)?;
        for index in 0..MAC_BYTES {
            self.out(PORT_NODE_ADDRESS_FIRST + index as u16, self.mac[index])?;
        }
        for index in 0..MAC_BYTES {
            if self.in_byte(PORT_NODE_ADDRESS_FIRST + index as u16)? != self.mac[index] {
                self.out(PORT_COMMAND, CR_HALT_PAGE_0)?;
                return Err(NicFault::ParMismatch);
            }
        }
        self.out(PORT_COMMAND, CR_HALT_PAGE_0)?;
        Ok(())
    }

    fn read_rx_header(&mut self, page: u8) -> Result<RxHeader, NicFault> {
        let mut words = [0u16; 2];
        self.remote_read((page as u16) << 8, &mut words)?;
        Ok(RxHeader {
            status: words[0] as u8,
            next_page: (words[0] >> 8) as u8,
            stored_length: words[1],
        })
    }

    fn read_rx_payload(&mut self, page: u8, payload_length: usize) -> Result<Vec<u8>, NicFault> {
        let start_address = ((page as u16) << 8) + RX_HEADER_BYTES as u16;
        let transfer_bytes = payload_length.next_multiple_of(2);
        let plan = plan_remote_read(start_address, transfer_bytes);
        let mut bytes = vec![0u8; transfer_bytes];
        self.remote_read_slice(plan.start_address, plan.direct_bytes, &mut bytes)?;
        if plan.wrapped_bytes > 0 {
            let mut tail = vec![0u8; plan.wrapped_bytes];
            self.remote_read_slice(RX_RING_START_ADDRESS, plan.wrapped_bytes, &mut tail)?;
            bytes[plan.direct_bytes..].copy_from_slice(&tail);
        }
        bytes.truncate(payload_length);
        Ok(bytes)
    }

    fn remote_read_slice(&mut self, address: u16, byte_count: usize, bytes: &mut [u8]) -> Result<(), NicFault> {
        let mut words = vec![0u16; byte_count / 2];
        self.remote_read(address, &mut words)?;
        for (index, word) in words.iter().enumerate() {
            bytes[index * 2] = *word as u8;
            bytes[index * 2 + 1] = (*word >> 8) as u8;
        }
        Ok(())
    }

    fn remote_read(&mut self, address: u16, words: &mut [u16]) -> Result<(), NicFault> {
        self.begin_remote_dma(address, words.len() * 2, CR_REMOTE_READ)?;
        self.bus
            .read_words(self.base + PORT_DATA_FIFO, words)
            .map_err(NicFault::Denied)?;
        self.out(PORT_COMMAND, CR_RUN_PAGE_0)
    }

    fn remote_write(&mut self, address: u16, words: &[u16]) -> Result<(), NicFault> {
        self.begin_remote_dma(address, words.len() * 2, CR_REMOTE_WRITE)?;
        self.bus
            .write_words(self.base + PORT_DATA_FIFO, words)
            .map_err(NicFault::Denied)?;
        self.poll_isr(
            ISR_REMOTE_DMA_COMPLETE,
            DMA_POLL_ATTEMPTS,
            NicFault::RemoteDmaTimeout,
        )?;
        Ok(())
    }

    fn begin_remote_dma(&mut self, address: u16, byte_count: usize, command: u8) -> Result<(), NicFault> {
        self.out(PORT_COMMAND, CR_RUN_PAGE_0)?;
        self.out(PORT_REMOTE_START0, address as u8)?;
        self.out(PORT_REMOTE_START1, (address >> 8) as u8)?;
        self.out(PORT_REMOTE_COUNT0, byte_count as u8)?;
        self.out(PORT_REMOTE_COUNT1, (byte_count >> 8) as u8)?;
        self.out(PORT_COMMAND, command)
    }

    fn poll_isr(&mut self, bits: u8, attempts: usize, timeout: NicFault) -> Result<u8, NicFault> {
        for _ in 0..attempts {
            let status = self.in_byte(PORT_INTERRUPT_STATUS)?;
            if status & bits != 0 {
                return Ok(status);
            }
            self.bus.sleep_ms(POLL_SLEEP_MS);
        }
        Err(timeout)
    }

    fn out(&mut self, offset: u16, value: u8) -> Result<(), NicFault> {
        self.bus
            .write_byte(self.base + offset, value)
            .map_err(NicFault::Denied)
    }

    fn in_byte(&mut self, offset: u16) -> Result<u8, NicFault> {
        self.bus.read_byte(self.base + offset).map_err(NicFault::Denied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u16 = ISA_DEFAULT_BASE;
    const CR_STOP_BIT: u8 = 0x01;
    const ISR_RX_PACKET: u8 = 0x01;
    const RX_STATUS_MULTICAST: u8 = 0x20;
    const RX_NEXT_PAGE_SPACING: usize = 0x100;

    const PROM_MAC: [u8; MAC_BYTES] = [0x52, 0x55, 0x00, 0x12, 0x34, 0x56];
    const FALLBACK_MAC: [u8; MAC_BYTES] = [0x02, 0x52, 0x4E, 0x45, 0x32, 0x03];

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    enum BusOp {
        Out(u16, u8),
        In(u16, u8),
        ReadWords(u16, usize),
        WriteWords(u16, usize),
        Sleep(u64),
    }

    fn out(offset: u16, value: u8) -> BusOp {
        BusOp::Out(BASE + offset, value)
    }

    fn is_node_address_port(offset: u16) -> bool {
        (PORT_NODE_ADDRESS_FIRST..PORT_NODE_ADDRESS_FIRST + MAC_BYTES as u16).contains(&offset)
    }

    fn read(offset: u16, value: u8) -> BusOp {
        BusOp::In(BASE + offset, value)
    }

    struct Ne2000Sim {
        sram: [u8; SRAM_TOP_ADDRESS as usize],
        cmd: u8,
        isr: u8,
        imr: u8,
        dcfg: u8,
        rxcr: u8,
        tcr: u8,
        start: u16,
        stop: u16,
        boundary: u8,
        tpsr: u8,
        tcnt: u16,
        tsr: u8,
        rsar: u16,
        rcnt: u16,
        curpag: u8,
        phys: [u8; MAC_BYTES],
        tx_fails: bool,
        accepts_writes_to_phys: bool,
        sent_frames: Vec<Vec<u8>>,
        transcript: Vec<BusOp>,
    }

    impl Default for Ne2000Sim {
        fn default() -> Self {
            Ne2000Sim {
                sram: [0; SRAM_TOP_ADDRESS as usize],
                cmd: CR_HALT_PAGE_0,
                isr: ISR_RESET,
                imr: 0,
                dcfg: 0,
                rxcr: 0,
                tcr: 0,
                start: RX_RING_START_ADDRESS,
                stop: (RX_RING_STOP_PAGE as u16) << 8,
                boundary: 0,
                tpsr: 0,
                tcnt: 0,
                tsr: 0,
                rsar: 0,
                rcnt: 0,
                curpag: 0,
                phys: [0; MAC_BYTES],
                tx_fails: false,
                accepts_writes_to_phys: true,
                sent_frames: Vec::new(),
                transcript: Vec::new(),
            }
        }
    }

    impl Ne2000Sim {
        fn seed_prom(&mut self, mac: &[u8; MAC_BYTES]) {
            for (index, byte) in mac.iter().enumerate() {
                self.sram[index * 2] = *byte;
                self.sram[index * 2 + 1] = *byte;
            }
        }

        fn seed_rx_page(
            &mut self,
            page: u8,
            status: u8,
            next_page: u8,
            total_length: u16,
            payload: &[u8],
        ) {
            let at = (page as usize) << 8;
            self.sram[at] = status;
            self.sram[at + 1] = next_page;
            self.sram[at + 2] = total_length as u8;
            self.sram[at + 3] = (total_length >> 8) as u8;
            self.sram[at + RX_HEADER_BYTES..at + RX_HEADER_BYTES + payload.len()]
                .copy_from_slice(payload);
        }

        fn deliver_frame(&mut self, frame: &[u8]) {
            if self.cmd & CR_STOP_BIT != 0 {
                return;
            }
            let ring = (self.stop - self.start) as usize;
            let mut index = (self.curpag as usize) << 8;
            let total_length = frame.len() + RX_HEADER_BYTES;
            let mut next = index + ((total_length + 4 + RX_NEXT_PAGE_SPACING - 1) & !0xFF);
            if next >= self.stop as usize {
                next -= ring;
            }
            let mut status = RX_HEADER_STATUS_RXOK;
            if frame[0] & 0x01 != 0 {
                status |= RX_STATUS_MULTICAST;
            }
            self.sram[index] = status;
            self.sram[index + 1] = (next >> 8) as u8;
            self.sram[index + 2] = total_length as u8;
            self.sram[index + 3] = (total_length >> 8) as u8;
            index += RX_HEADER_BYTES;
            let mut remainder = frame;
            while !remainder.is_empty() {
                let available = self.stop as usize - index;
                let chunk = remainder.len().min(available);
                self.sram[index..index + chunk].copy_from_slice(&remainder[..chunk]);
                remainder = &remainder[chunk..];
                index += chunk;
                if index == self.stop as usize {
                    index = self.start as usize;
                }
            }
            self.curpag = (next >> 8) as u8;
            self.isr |= ISR_RX_PACKET;
        }

        fn dma_update(&mut self, amount: u16) {
            self.rsar += amount;
            if self.rsar == self.stop {
                self.rsar = self.start;
            }
            if self.rcnt <= amount {
                self.rcnt = 0;
                self.isr |= ISR_REMOTE_DMA_COMPLETE;
            } else {
                self.rcnt -= amount;
            }
        }

        fn register_write(&mut self, offset: u16, value: u8) {
            if offset == PORT_COMMAND {
                self.cmd = value;
                if value & CR_STOP_BIT == 0 {
                    self.isr &= !ISR_RESET;
                    if value & (CR_REMOTE_READ | CR_REMOTE_WRITE) != 0 && self.rcnt == 0 {
                        self.isr |= ISR_REMOTE_DMA_COMPLETE;
                    }
                    if value & CR_TRANSMIT_COMMAND_BIT != 0 {
                        let start = (self.tpsr as usize) << 8;
                        self.sent_frames
                            .push(self.sram[start..start + self.tcnt as usize].to_vec());
                        if self.tx_fails {
                            self.tsr = 0x20;
                            self.isr |= ISR_TX_ERROR;
                        } else {
                            self.tsr = 0x01;
                            self.isr |= ISR_TX_COMPLETE;
                        }
                        self.cmd &= !CR_TRANSMIT_COMMAND_BIT;
                    }
                }
                return;
            }
            match (self.cmd >> 6, offset) {
                (0, PORT_PAGE_LOW) => self.start = (value as u16) << 8,
                (0, PORT_PAGE_HIGH) => self.stop = (value as u16) << 8,
                (0, PORT_BOUNDARY) => self.boundary = value,
                (0, PORT_TRANSMIT_STATUS) => self.tpsr = value,
                (0, PORT_TRANSMIT_COUNT0) => self.tcnt = (self.tcnt & 0xFF00) | value as u16,
                (0, PORT_TRANSMIT_COUNT1) => self.tcnt = (self.tcnt & 0x00FF) | ((value as u16) << 8),
                (0, PORT_REMOTE_START0) => self.rsar = (self.rsar & 0xFF00) | value as u16,
                (0, PORT_REMOTE_START1) => self.rsar = (self.rsar & 0x00FF) | ((value as u16) << 8),
                (0, PORT_REMOTE_COUNT0) => self.rcnt = (self.rcnt & 0xFF00) | value as u16,
                (0, PORT_REMOTE_COUNT1) => self.rcnt = (self.rcnt & 0x00FF) | ((value as u16) << 8),
                (0, PORT_RECEIVE_CONFIG) => self.rxcr = value,
                (0, PORT_TRANSMIT_CONFIG) => self.tcr = value,
                (0, PORT_DATA_CONFIG) => self.dcfg = value,
                (0, PORT_INTERRUPT_STATUS) => self.isr &= !(value & 0x7F),
                (0, PORT_INTERRUPT_MASK) => self.imr = value,
                (1, offset) if is_node_address_port(offset) && self.accepts_writes_to_phys => {
                    self.phys[offset as usize - PORT_NODE_ADDRESS_FIRST as usize] = value;
                }
                (1, 7) => self.curpag = value,
                _ => {}
            }
        }

        fn register_read(&self, offset: u16) -> u8 {
            if offset == PORT_COMMAND {
                return self.cmd;
            }
            match (self.cmd >> 6, offset) {
                (0, PORT_INTERRUPT_STATUS) => self.isr,
                (0, PORT_TRANSMIT_STATUS) => self.tsr,
                (0, PORT_BOUNDARY) => self.boundary,
                (1, offset) if is_node_address_port(offset) => {
                    self.phys[offset as usize - PORT_NODE_ADDRESS_FIRST as usize]
                }
                (1, 7) => self.curpag,
                _ => 0,
            }
        }

        fn writes_since(&self, mark: usize) -> Vec<BusOp> {
            self.transcript[mark..]
                .iter()
                .copied()
                .filter(|op| !matches!(op, BusOp::In(..) | BusOp::Sleep(..)))
                .collect()
        }
    }

    impl NicBus for Ne2000Sim {
        fn read_byte(&mut self, port: u16) -> Result<u8, PortError> {
            let value = self.register_read(port - BASE);
            self.transcript.push(BusOp::In(port, value));
            Ok(value)
        }

        fn write_byte(&mut self, port: u16, value: u8) -> Result<(), PortError> {
            self.transcript.push(BusOp::Out(port, value));
            self.register_write(port - BASE, value);
            Ok(())
        }

        fn read_words(&mut self, port: u16, destination: &mut [u16]) -> Result<(), PortError> {
            self.transcript.push(BusOp::ReadWords(port, destination.len()));
            for slot in destination.iter_mut() {
                let at = self.rsar as usize;
                *slot = u16::from_le_bytes([self.sram[at], self.sram[at + 1]]);
                self.dma_update(2);
            }
            Ok(())
        }

        fn write_words(&mut self, port: u16, source: &[u16]) -> Result<(), PortError> {
            self.transcript.push(BusOp::WriteWords(port, source.len()));
            for word in source {
                let at = self.rsar as usize;
                self.sram[at] = *word as u8;
                self.sram[at + 1] = (word >> 8) as u8;
                self.dma_update(2);
            }
            Ok(())
        }

        fn sleep_ms(&mut self, millis: u64) {
            self.transcript.push(BusOp::Sleep(millis));
        }
    }

    struct DeadBus;

    impl NicBus for DeadBus {
        fn read_byte(&mut self, _port: u16) -> Result<u8, PortError> {
            Ok(0)
        }

        fn write_byte(&mut self, _port: u16, _value: u8) -> Result<(), PortError> {
            Ok(())
        }

        fn read_words(&mut self, _port: u16, _destination: &mut [u16]) -> Result<(), PortError> {
            Ok(())
        }

        fn write_words(&mut self, _port: u16, _source: &[u16]) -> Result<(), PortError> {
            Ok(())
        }

        fn sleep_ms(&mut self, _millis: u64) {}
    }

    fn configured_sim() -> Ne2000Sim {
        let mut sim = Ne2000Sim::default();
        sim.seed_prom(&PROM_MAC);
        sim
    }

    fn initialized(nic: &mut Ne2k<Ne2000Sim>) {
        assert_eq!(nic.initialize(), Ok(PROM_MAC));
    }

    fn sample_frame(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index % 251) as u8).collect()
    }

    #[test]
    fn initialize_runs_the_canonical_programming_sequence() {
        let mut nic = Ne2k::new(BASE, configured_sim());
        initialized(&mut nic);

        let expected = vec![
            out(PORT_COMMAND, CR_HALT_PAGE_0),
            read(PORT_INTERRUPT_STATUS, ISR_RESET),
            out(PORT_INTERRUPT_STATUS, ISR_RESET),
            out(PORT_DATA_CONFIG, DCR_16_BIT_LONG_TIMING),
            out(PORT_PAGE_LOW, RX_RING_FIRST_PAGE),
            out(PORT_PAGE_HIGH, RX_RING_STOP_PAGE),
            out(PORT_BOUNDARY, RX_RING_FIRST_PAGE),
            out(PORT_COMMAND, CR_HALT_PAGE_1),
            out(PORT_INTERRUPT_STATUS, RX_RING_FIRST_PAGE + 1),
            out(PORT_COMMAND, CR_HALT_PAGE_0),
            out(PORT_COMMAND, CR_RUN_PAGE_0),
            out(PORT_REMOTE_START0, 0x00),
            out(PORT_REMOTE_START1, 0x00),
            out(PORT_REMOTE_COUNT0, 0x10),
            out(PORT_REMOTE_COUNT1, 0x00),
            out(PORT_COMMAND, CR_REMOTE_READ),
            BusOp::ReadWords(BASE + PORT_DATA_FIFO, 8),
            out(PORT_COMMAND, CR_RUN_PAGE_0),
            out(PORT_COMMAND, CR_HALT_PAGE_1),
            out(PORT_NODE_ADDRESS_FIRST + 0, PROM_MAC[0]),
            out(PORT_NODE_ADDRESS_FIRST + 1, PROM_MAC[1]),
            out(PORT_NODE_ADDRESS_FIRST + 2, PROM_MAC[2]),
            out(PORT_NODE_ADDRESS_FIRST + 3, PROM_MAC[3]),
            out(PORT_NODE_ADDRESS_FIRST + 4, PROM_MAC[4]),
            out(PORT_NODE_ADDRESS_FIRST + 5, PROM_MAC[5]),
            read(PORT_NODE_ADDRESS_FIRST + 0, PROM_MAC[0]),
            read(PORT_NODE_ADDRESS_FIRST + 1, PROM_MAC[1]),
            read(PORT_NODE_ADDRESS_FIRST + 2, PROM_MAC[2]),
            read(PORT_NODE_ADDRESS_FIRST + 3, PROM_MAC[3]),
            read(PORT_NODE_ADDRESS_FIRST + 4, PROM_MAC[4]),
            read(PORT_NODE_ADDRESS_FIRST + 5, PROM_MAC[5]),
            out(PORT_COMMAND, CR_HALT_PAGE_0),
            out(PORT_RECEIVE_CONFIG, RCR_PHYSICAL_MULTICAST_BROADCAST),
            out(PORT_TRANSMIT_CONFIG, TCR_INTERNAL_LOOPBACK),
            out(PORT_INTERRUPT_STATUS, ISR_CLEAR_ALL),
            out(PORT_INTERRUPT_MASK, IMR_ENABLE_EVENTS),
            out(PORT_COMMAND, CR_RUN_PAGE_0),
            out(PORT_TRANSMIT_CONFIG, TCR_NORMAL),
        ];
        assert_eq!(nic.bus.transcript, expected);
        assert_eq!(nic.bus.phys, PROM_MAC);
        assert_eq!(nic.mac(), PROM_MAC);
        assert_eq!(nic.counters(), NicCounters::default());
    }

    #[test]
    fn initialize_falls_back_when_the_prom_never_was_programmed() {
        for blank in [0x00u8, 0xFF] {
            let mut sim = Ne2000Sim::default();
            sim.seed_prom(&[blank; MAC_BYTES]);
            let mut nic = Ne2k::new(BASE, sim);
            assert_eq!(nic.initialize(), Ok(FALLBACK_MAC));
            assert_eq!(nic.bus.phys, FALLBACK_MAC);
        }
    }

    #[test]
    fn initialize_reports_absent_when_reset_never_completes() {
        let mut sim = Ne2000Sim::default();
        sim.isr = 0;
        let mut nic = Ne2k::new(BASE, sim);

        assert_eq!(nic.initialize(), Err(NicFault::Absent));

        let mut expected = vec![out(PORT_COMMAND, CR_HALT_PAGE_0)];
        for _ in 0..RESET_POLL_ATTEMPTS {
            expected.push(read(PORT_INTERRUPT_STATUS, 0));
            expected.push(BusOp::Sleep(POLL_SLEEP_MS));
        }
        assert_eq!(nic.bus.transcript, expected);
    }

    #[test]
    fn initialize_reports_par_mismatch_when_the_nic_refuses_the_address() {
        let mut sim = configured_sim();
        sim.accepts_writes_to_phys = false;
        let mut nic = Ne2k::new(BASE, sim);

        assert_eq!(nic.initialize(), Err(NicFault::ParMismatch));

        let last = *nic.bus.transcript.last().unwrap();
        assert_eq!(last, out(PORT_COMMAND, CR_HALT_PAGE_0));
    }

    #[test]
    fn transmit_walks_the_remote_write_and_command_sequence() {
        let mut nic = Ne2k::new(BASE, configured_sim());
        initialized(&mut nic);
        let frame = sample_frame(ETHERNET_MIN_BYTES);

        assert_eq!(nic.transmit(&frame), Ok(()));

        let expected = vec![
            read(PORT_COMMAND, CR_RUN_PAGE_0),
            out(PORT_COMMAND, CR_RUN_PAGE_0),
            out(PORT_REMOTE_START0, TX_BUFFER_ADDRESS as u8),
            out(PORT_REMOTE_START1, (TX_BUFFER_ADDRESS >> 8) as u8),
            out(PORT_REMOTE_COUNT0, ETHERNET_MIN_BYTES as u8),
            out(PORT_REMOTE_COUNT1, 0),
            out(PORT_COMMAND, CR_REMOTE_WRITE),
            BusOp::WriteWords(BASE + PORT_DATA_FIFO, ETHERNET_MIN_BYTES / 2),
            read(PORT_INTERRUPT_STATUS, ISR_REMOTE_DMA_COMPLETE),
            out(PORT_COMMAND, CR_RUN_PAGE_0),
            out(PORT_TRANSMIT_STATUS, TX_BUFFER_FIRST_PAGE),
            out(PORT_TRANSMIT_COUNT0, ETHERNET_MIN_BYTES as u8),
            out(PORT_TRANSMIT_COUNT1, 0),
            out(PORT_COMMAND, CR_TRANSMIT),
            read(PORT_INTERRUPT_STATUS, ISR_TX_COMPLETE | ISR_REMOTE_DMA_COMPLETE),
            read(PORT_TRANSMIT_STATUS, 0x01),
            out(PORT_INTERRUPT_STATUS, ISR_TX_COMPLETE | ISR_REMOTE_DMA_COMPLETE),
        ];
        let transcript = &nic.bus.transcript;
        assert_eq!(transcript[transcript.len() - expected.len()..], expected);
        assert_eq!(nic.bus.sent_frames, vec![frame]);
        assert_eq!(nic.counters().transmitted_frames, 1);
        assert_eq!(nic.counters().transmit_errors, 0);
    }

    #[test]
    fn transmit_pads_odd_frames_for_word_writes_and_keeps_the_true_length() {
        let mut nic = Ne2k::new(BASE, configured_sim());
        initialized(&mut nic);
        let frame = sample_frame(61);

        assert_eq!(nic.transmit(&frame), Ok(()));

        let writes = nic.bus.writes_since(0);
        assert!(writes.contains(&out(PORT_REMOTE_COUNT0, 62)));
        assert!(writes.contains(&out(PORT_TRANSMIT_COUNT0, 61)));
        assert!(writes.contains(&BusOp::WriteWords(BASE + PORT_DATA_FIFO, 31)));
        assert_eq!(nic.bus.sent_frames, vec![frame]);
        assert_eq!(nic.bus.sram[TX_BUFFER_ADDRESS as usize + 61], 0);
    }

    #[test]
    fn transmit_refuses_while_a_transmission_is_in_flight() {
        let mut nic = Ne2k::new(BASE, configured_sim());
        initialized(&mut nic);
        nic.bus.cmd |= CR_TRANSMIT_COMMAND_BIT;
        let mark = nic.bus.transcript.len();

        assert_eq!(nic.transmit(&sample_frame(60)), Err(NicFault::TransmitBusy));

        assert_eq!(
            nic.bus.transcript[mark..],
            [read(PORT_COMMAND, CR_RUN_PAGE_0 | CR_TRANSMIT_COMMAND_BIT)]
        );
        assert!(nic.bus.sent_frames.is_empty());
    }

    #[test]
    fn transmit_reports_device_failure() {
        let mut nic = Ne2k::new(BASE, configured_sim());
        initialized(&mut nic);
        nic.bus.tx_fails = true;

        assert_eq!(nic.transmit(&sample_frame(60)), Err(NicFault::TransmitFailed));
        assert_eq!(nic.counters().transmit_errors, 1);
        assert_eq!(nic.counters().transmitted_frames, 0);
    }

    #[test]
    fn transmit_refuses_frames_outside_the_ethernet_bounds() {
        let mut nic = Ne2k::new(BASE, configured_sim());
        initialized(&mut nic);
        let mark = nic.bus.transcript.len();

        assert_eq!(nic.transmit(&[]), Err(NicFault::Oversize));
        assert_eq!(
            nic.transmit(&sample_frame(ETHERNET_MAX_BYTES + 1)),
            Err(NicFault::Oversize)
        );

        assert!(nic.bus.transcript[mark..].is_empty());
    }

    #[test]
    fn transmit_reports_a_remote_write_timeout() {
        let mut nic = Ne2k::new(BASE, DeadBus);

        assert_eq!(nic.transmit(&sample_frame(60)), Err(NicFault::RemoteDmaTimeout));
    }

    #[test]
    fn receive_reads_a_frame_and_then_answers_empty() {
        let mut nic = Ne2k::new(BASE, configured_sim());
        initialized(&mut nic);
        let frame = sample_frame(ETHERNET_MIN_BYTES);
        nic.bus.deliver_frame(&frame);
        let mark = nic.bus.transcript.len();

        assert_eq!(nic.receive(), Ok(Some(frame.clone())));

        let expected = vec![
            out(PORT_COMMAND, CR_RUN_PAGE_1),
            out(PORT_COMMAND, CR_RUN_PAGE_0),
            out(PORT_COMMAND, CR_RUN_PAGE_0),
            out(PORT_REMOTE_START0, 0x00),
            out(PORT_REMOTE_START1, RX_RING_FIRST_PAGE + 1),
            out(PORT_REMOTE_COUNT0, RX_HEADER_BYTES as u8),
            out(PORT_REMOTE_COUNT1, 0),
            out(PORT_COMMAND, CR_REMOTE_READ),
            BusOp::ReadWords(BASE + PORT_DATA_FIFO, 2),
            out(PORT_COMMAND, CR_RUN_PAGE_0),
            out(PORT_BOUNDARY, RX_RING_FIRST_PAGE + 1),
            out(PORT_COMMAND, CR_RUN_PAGE_0),
            out(PORT_REMOTE_START0, RX_HEADER_BYTES as u8),
            out(PORT_REMOTE_START1, RX_RING_FIRST_PAGE + 1),
            out(PORT_REMOTE_COUNT0, ETHERNET_MIN_BYTES as u8),
            out(PORT_REMOTE_COUNT1, 0),
            out(PORT_COMMAND, CR_REMOTE_READ),
            BusOp::ReadWords(BASE + PORT_DATA_FIFO, ETHERNET_MIN_BYTES / 2),
            out(PORT_COMMAND, CR_RUN_PAGE_0),
        ];
        assert_eq!(nic.bus.writes_since(mark), expected);
        assert_eq!(nic.counters().received_frames, 1);

        assert_eq!(nic.receive(), Ok(None));
        assert_eq!(nic.counters().received_frames, 1);
    }

    #[test]
    fn receive_splits_a_frame_at_the_ring_top_and_resumes_at_the_ring_start() {
        let mut nic = Ne2k::new(BASE, configured_sim());
        initialized(&mut nic);
        nic.rx_page = RX_RING_LAST_PAGE - 2;
        nic.bus.curpag = RX_RING_LAST_PAGE - 2;
        let frame = sample_frame(ETHERNET_MAX_BYTES);
        nic.bus.deliver_frame(&frame);

        assert_eq!(nic.receive(), Ok(Some(frame)));

        let writes = nic.bus.writes_since(0);
        let payload_page = RX_RING_LAST_PAGE - 2;
        let direct_bytes = SRAM_TOP_ADDRESS - ((payload_page as u16) << 8) - RX_HEADER_BYTES as u16;
        let wrapped_bytes = ETHERNET_MAX_BYTES as u16 - direct_bytes;
        assert!(writes.contains(&out(PORT_REMOTE_COUNT0, direct_bytes as u8)));
        assert!(writes.contains(&out(PORT_REMOTE_COUNT1, (direct_bytes >> 8) as u8)));
        let resume_at = writes
            .iter()
            .rposition(|op| *op == out(PORT_REMOTE_START1, RX_RING_FIRST_PAGE))
            .expect("wrap resumes at the ring start");
        assert_eq!(writes[resume_at - 1], out(PORT_REMOTE_START0, 0x00));
        assert_eq!(writes[resume_at + 1], out(PORT_REMOTE_COUNT0, wrapped_bytes as u8));
        assert_eq!(writes[resume_at + 2], out(PORT_REMOTE_COUNT1, (wrapped_bytes >> 8) as u8));
        assert_eq!(nic.counters().received_frames, 1);
    }

    #[test]
    fn receive_skips_headers_with_error_status_or_bad_bounds() {
        struct HeaderCase {
            label: &'static str,
            status: u8,
            next_page: u8,
            total_length: u16,
            expect_error: bool,
        }
        let next = RX_RING_FIRST_PAGE + 2;
        let cases = [
            HeaderCase { label: "missing rxok", status: 0x00, next_page: next, total_length: 64, expect_error: true },
            HeaderCase { label: "fifo overrun bit", status: RX_HEADER_STATUS_RXOK | 0x08, next_page: next, total_length: 64, expect_error: true },
            HeaderCase { label: "undersize total", status: RX_HEADER_STATUS_RXOK, next_page: next, total_length: 63, expect_error: true },
            HeaderCase { label: "oversize total", status: RX_HEADER_STATUS_RXOK, next_page: next, total_length: 1519, expect_error: true },
            HeaderCase { label: "next page off the ring", status: RX_HEADER_STATUS_RXOK, next_page: 0x00, total_length: 64, expect_error: true },
            HeaderCase { label: "multicast-flagged good frame", status: RX_HEADER_STATUS_RXOK | RX_STATUS_MULTICAST, next_page: next, total_length: 64, expect_error: false },
        ];
        for case in cases {
            let mut nic = Ne2k::new(BASE, configured_sim());
            initialized(&mut nic);
            nic.bus.seed_rx_page(
                RX_RING_FIRST_PAGE + 1,
                case.status,
                case.next_page,
                case.total_length,
                &sample_frame(60),
            );
            nic.bus.curpag = next;

            if case.expect_error {
                assert_eq!(nic.receive(), Ok(None), "{}", case.label);
                assert_eq!(nic.counters().receive_errors, 1, "{}", case.label);
                assert_eq!(nic.counters().received_frames, 0, "{}", case.label);
            } else {
                assert_eq!(nic.receive(), Ok(Some(sample_frame(60))), "{}", case.label);
                assert_eq!(nic.counters().received_frames, 1, "{}", case.label);
            }
        }
    }

    #[test]
    fn recover_overrun_restarts_the_ring_and_counts_it() {
        let mut nic = Ne2k::new(BASE, configured_sim());
        initialized(&mut nic);
        nic.bus.curpag = RX_RING_FIRST_PAGE + 5;
        let mark = nic.bus.transcript.len();

        assert_eq!(nic.recover_overrun(), Ok(()));

        let expected = vec![
            out(PORT_BOUNDARY, RX_RING_FIRST_PAGE),
            out(PORT_COMMAND, CR_RUN_PAGE_1),
            out(PORT_INTERRUPT_STATUS, RX_RING_FIRST_PAGE + 1),
            out(PORT_COMMAND, CR_RUN_PAGE_0),
        ];
        assert_eq!(nic.bus.writes_since(mark), expected);
        assert_eq!(nic.counters().receive_overruns, 1);
        assert_eq!(nic.bus.curpag, RX_RING_FIRST_PAGE + 1);
        assert_eq!(nic.receive(), Ok(None));
    }

    #[test]
    fn clear_interrupt_flags_acks_exactly_what_it_saw() {
        let mut nic = Ne2k::new(BASE, configured_sim());
        initialized(&mut nic);
        nic.bus.isr = ISR_RESET | ISR_RX_PACKET | ISR_REMOTE_DMA_COMPLETE;

        let status = nic.clear_interrupt_flags().unwrap();

        assert_eq!(status, ISR_RESET | ISR_RX_PACKET | ISR_REMOTE_DMA_COMPLETE);
        assert_eq!(nic.bus.isr, ISR_RESET);
    }

    #[test]
    fn frame_padding_maps_odd_tails_into_the_high_word_byte() {
        assert_eq!(frame_to_padded_words(b"abcd"), vec![0x6261, 0x6463]);
        assert_eq!(frame_to_padded_words(b"abcde"), vec![0x6261, 0x6463, 0x0065]);
    }
}
