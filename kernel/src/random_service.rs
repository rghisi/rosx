#[cfg(not(test))]
use system::ipc::{Message, endpoint, random};
#[cfg(not(test))]
use crate::ipc::kernel_ipc_recv_blocking;
#[cfg(not(test))]
use crate::kernel_services::services;

pub(crate) struct Xorshift64(u64);

impl Xorshift64 {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[cfg(not(test))]
pub fn random_server() {
    services().endpoint_registry.borrow_mut().create(endpoint::RANDOM).ok();
    let mut rng = Xorshift64::new(0xDEAD_BEEF_CAFE_BABE);
    loop {
        let (token, _msg) = kernel_ipc_recv_blocking(endpoint::RANDOM);
        let value = rng.next();
        let reply = Message::new(random::TAG_VALUE).with_word(0, value);
        services().endpoint_registry.borrow_mut().reply(token, reply);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_nonzero_for_nonzero_seed() {
        let mut rng = Xorshift64::new(1);
        assert_ne!(rng.next(), 0);
    }

    #[test]
    fn consecutive_outputs_differ() {
        let mut rng = Xorshift64::new(12345);
        let a = rng.next();
        let b = rng.next();
        assert_ne!(a, b);
    }

    #[test]
    fn same_seed_produces_same_sequence() {
        let mut r1 = Xorshift64::new(99);
        let mut r2 = Xorshift64::new(99);
        assert_eq!(r1.next(), r2.next());
        assert_eq!(r1.next(), r2.next());
    }

    #[test]
    fn different_seeds_produce_different_outputs() {
        let mut r1 = Xorshift64::new(1);
        let mut r2 = Xorshift64::new(2);
        assert_ne!(r1.next(), r2.next());
    }
}
