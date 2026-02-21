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

const WIDTH: usize = 10;
const HEIGHT: usize = 20;

struct Board {
    cells: [[u8; WIDTH]; HEIGHT],
}

impl Board {
    fn new() -> Self {
        Self { cells: [[0; WIDTH]; HEIGHT] }
    }
}

fn cell_color(index: u8) -> &'static str {
    match index {
        1 => "\x1B[46m ",
        2 => "\x1B[43m ",
        3 => "\x1B[45m ",
        4 => "\x1B[42m ",
        5 => "\x1B[41m ",
        6 => "\x1B[44m ",
        7 => "\x1B[47m ",
        _ => " ",
    }
}

fn render(board: &Board, score: usize, lines: usize) {
    print!("\x1B[H");
    println!("\x1B[97mTETRIS\x1B[m  Score: {:<6}  Lines: {:<4}  WASD=move  Q=quit", score, lines);

    print!("+");
    for _ in 0..WIDTH {
        print!("--");
    }
    println!("+");

    for row in 0..HEIGHT {
        print!("|");
        for col in 0..WIDTH {
            print!("{}\x1B[m", cell_color(board.cells[row][col]));
            print!(" ");
        }
        println!("|");
    }

    print!("+");
    for _ in 0..WIDTH {
        print!("--");
    }
    println!("+");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    print!("\x1B[2J\x1B[H");

    let board = Board::new();
    render(&board, 0, 0);

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
