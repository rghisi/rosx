#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::panic::PanicInfo;
use usrlib::{print, println};
use usrlib::syscall::Syscall;

struct SyscallAllocator;

unsafe impl GlobalAlloc for SyscallAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Syscall::alloc(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        Syscall::dealloc(ptr, layout.size(), layout.align());
    }
}

#[global_allocator]
static ALLOCATOR: SyscallAllocator = SyscallAllocator;

const DIGITS: usize = 20000;

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    // We need a slightly larger array to account for the mathematical bounds.
    // The size roughly correlates to (10/3) * N.
    let n = DIGITS;
    let len = (n * 10) / 3 + 1;

    // The "rem" (remainder) vector holds the internal state of the spigot.
    // We initialize it with 2, because the algorithm starts from the
    // expansion: Pi = 2 + 1/3 * (2 + 2/5 * (2 + 3/7 * (...)))
    let mut rem: Vec<u64> = vec![2; len];

    // 'predigit' holds the digit we are about to print, waiting to see
    // if a "9" ripple effect forces us to increment it.
    let mut predigit: Option<u64> = None;
    let mut nines = 0;

    print!("Pi: ");

    // Loop to generate digits
    for j in 0..n {
        let mut q = 0;

        // Working backwards through the array to calculate the next digit.
        // This is the core "base conversion" logic of the spigot algorithm.
        for i in (1..len).rev() {
            // The formula derived from the mixed-radix representation:
            // x = 10 * remainder + carry * index
            let x = 10 * rem[i] + q * (i as u64);

            // The denominator for the mixed radix base is (2 * i + 1)
            let denom = 2 * (i as u64) + 1;

            rem[i] = x % denom;
            q = x / denom;
        }

        // Handle the final step at i=0.
        // x = 10 * rem[0] + q * 0, so just 10 * rem[0] + q in practice,
        // but rem[0] isn't strictly part of the base logic in the same way.
        // In this implementation, the carry 'q' falling out of the loop
        // *is* the next raw digit information.
        let x = 10 * rem[0] + q;
        rem[0] = x % 10;
        q = x / 10;

        // --- Output Handling (The "Nines" Logic) ---
        // Spigot algorithms can produce "tentative" digits.
        // If we get a 9, it might become a 0 (and increment the previous digit)
        // if the *next* digit overflows.

        if q == 9 {
            nines += 1;
        } else if q == 10 {
            // Overflow! The nines roll over to zeros, and predigit increments.
            if let Some(d) = predigit {
                print!("{}", d + 1);
            }
            for _ in 0..nines {
                print!("0");
            }
            predigit = Some(0);
            nines = 0;
        } else {
            // Safe digit (0-8). We can print the held predigit and nines.
            if let Some(d) = predigit {
                print!("{}", d);
                // Print the decimal point after the very first digit (3)
                if j == 1 { print!("."); }
            }
            for _ in 0..nines {
                print!("9");
            }
            predigit = Some(q);
            nines = 0;
        }

        // Flush stdout to see digits appear in real-time
        // io::stdout().flush().unwrap();
    }
    println!();

}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
