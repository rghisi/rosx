use usrlib::println;
use usrlib::syscall::Syscall;

static mut COUNT: usize = 0;
pub fn main() {
    let c = unsafe { COUNT };
    unsafe {
        COUNT += 1;
    };
    println!("\nshell {} started", c);
    Syscall::exec(main as usize);
    Syscall::sleep(500);
    println!("\nclosing shell {}", c);
}
