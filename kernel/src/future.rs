use syscall;

pub trait Future {
    fn is_completed(&self) -> bool;

    fn complete(&mut self);
}

pub struct TimeFuture {
    completion_timestamp: u64
}

impl TimeFuture {
   pub fn new(ms: u64) -> TimeFuture {
        TimeFuture {
            completion_timestamp: syscall::get_system_time() + ms,
        }
    }
}
impl Future for TimeFuture {

    fn is_completed(&self) -> bool {
        syscall::get_system_time() > self.completion_timestamp
    }

    fn complete(&mut self) {

    }
}