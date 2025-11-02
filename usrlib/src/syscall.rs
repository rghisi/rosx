pub struct Syscall {}
impl Syscall {

    pub fn exec(entrypoint: usize) {
        kernel::kernel::exec(entrypoint);
    }

    pub fn wait() {

    }

    pub fn task_yield() {

    }
}