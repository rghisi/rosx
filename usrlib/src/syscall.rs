use system::message::Message;

pub struct Syscall {}

impl Syscall {

    pub fn exec(entrypoint: usize) {
        kernel::syscall::exec(entrypoint);
    }

    pub fn syscall(message: &Message) -> usize{
        kernel::syscall::syscall(message)
    }

    pub fn task_yield() {

    }
}