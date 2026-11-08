use alloc::string::String;
use alloc::vec::Vec;
use block_protocol::client::{Block, ClientError};
use block_protocol::wire::BlockError;

pub const ATA_SERVICE: &str = "ATA";
pub const ATA_SOURCE: &str = "ata0";

pub struct ProviderGeometry {
    pub block_count: u64,
    pub block_size: u32,
    pub description: String,
}

pub trait Provider {
    fn probe(&mut self, drive: u32) -> Result<ProviderGeometry, BlockError>;
    fn read(&mut self, drive: u32, block: u64, count: u16) -> Result<Vec<u8>, BlockError>;
    fn write(&mut self, drive: u32, block: u64, count: u16, data: &[u8]) -> Result<(), BlockError>;
    fn flush(&mut self, drive: u32) -> Result<(), BlockError>;
}

pub struct AtaProvider {
    client: Block,
}

impl AtaProvider {
    pub fn dial(service: &str) -> Result<AtaProvider, BlockError> {
        Block::connect(service).map(|client| AtaProvider { client }).map_err(provider_fault)
    }
}

fn provider_fault(error: ClientError) -> BlockError {
    match error {
        ClientError::Block(error) => error,
        _ => BlockError::Device,
    }
}

impl Provider for AtaProvider {
    fn probe(&mut self, drive: u32) -> Result<ProviderGeometry, BlockError> {
        self.client.probe(drive).map(|geometry| ProviderGeometry {
            block_count: geometry.block_count,
            block_size: geometry.block_size,
            description: geometry.description,
        }).map_err(provider_fault)
    }

    fn read(&mut self, drive: u32, block: u64, count: u16) -> Result<Vec<u8>, BlockError> {
        self.client.read_blocks(drive, block, count as usize).map_err(provider_fault)
    }

    fn write(&mut self, drive: u32, block: u64, count: u16, data: &[u8]) -> Result<(), BlockError> {
        self.client.write_blocks(drive, block, count as usize, data).map_err(provider_fault)
    }

    fn flush(&mut self, drive: u32) -> Result<(), BlockError> {
        self.client.flush(drive).map_err(provider_fault)
    }
}
