use alloc::boxed::Box;
use alloc::vec::Vec;
pub struct InputEvents {
    subscribers: Vec<Box<dyn InputEventSubscriber>>,
}

#[derive(Copy, Clone)]
pub enum InputEvent {
    Keyboard,
}
impl InputEvents {
    pub fn new() -> Self {
        InputEvents {
            subscribers: Vec::with_capacity(2),
        }
    }
    pub fn publish(&mut self, message: InputEvent) {
        for subscriber in &mut self.subscribers {
            subscriber.receive(message);
        }
    }

    pub fn subscribe(&mut self, subscriber: Box<dyn InputEventSubscriber>) {}
}

pub trait InputEventSubscriber {
    fn receive(&mut self, event: InputEvent);
}
