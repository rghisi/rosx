#![no_std]
#![no_main]

extern crate alloc;

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

const COLS: usize = 80;
const ROWS: usize = 23;

struct Grid {
    cells: [[bool; COLS]; ROWS],
}

impl Grid {
    fn new() -> Self {
        Self { cells: [[false; COLS]; ROWS] }
    }
}

fn render(grid: &Grid, generation: usize, population: usize, paused: bool) {
    print!("\x1B[H");

    if paused {
        println!("\x1B[97mCONWAY\x1B[m  Gen: {:<6}  Pop: {:<6}  \x1B[93mPAUSED\x1B[m  Space=pause  R=randomize  Q=quit",
            generation, population);
    } else {
        println!("\x1B[97mCONWAY\x1B[m  Gen: {:<6}  Pop: {:<6}  Space=pause  R=randomize  Q=quit          ",
            generation, population);
    }

    for row in 0..ROWS {
        for col in 0..COLS {
            if grid.cells[row][col] {
                print!("\x1B[42m \x1B[m");
            } else {
                print!(" ");
            }
        }
        if row < ROWS - 1 {
            println!();
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    print!("\x1B[2J\x1B[H");

    let grid = Grid::new();
    render(&grid, 0, 0, false);

    loop {
        match Syscall::read_char() {
            'q' | 'Q' => return,
            _ => {}
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
