use usrlib::println;
use usrlib::syscall::Syscall;

pub fn run() {
    println!("[RandomService] Starting...");
    let a = Syscall::random_next_u64();
    let b = Syscall::random_next_u64();
    assert_ne!(a, 0, "random value must be non-zero");
    assert_ne!(a, b, "consecutive random values must differ");
    println!("[RandomService] PASSED (a={:#x}, b={:#x})", a, b);
}
