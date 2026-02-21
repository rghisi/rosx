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

struct TetrominoType {
    rotations: [u16; 4],
    color: u8,
}

const TETROMINOES: [TetrominoType; 7] = [
    TetrominoType { rotations: [0x0F00, 0x2222, 0x00F0, 0x4444], color: 1 }, // I - cyan
    TetrominoType { rotations: [0x0660, 0x0660, 0x0660, 0x0660], color: 2 }, // O - yellow
    TetrominoType { rotations: [0x0E40, 0x4C40, 0x4E00, 0x4640], color: 3 }, // T - magenta
    TetrominoType { rotations: [0x06C0, 0x8C40, 0x06C0, 0x8C40], color: 4 }, // S - green
    TetrominoType { rotations: [0x0C60, 0x4C80, 0x0C60, 0x4C80], color: 5 }, // Z - red
    TetrominoType { rotations: [0x8E00, 0x6440, 0x0E20, 0x44C0], color: 6 }, // J - blue
    TetrominoType { rotations: [0x2E00, 0x4460, 0x0E80, 0xC440], color: 7 }, // L - white
];

struct Piece {
    kind: usize,
    rotation: usize,
    col: i32,
    row: i32,
}

impl Piece {
    fn new(kind: usize) -> Self {
        Self { kind, rotation: 0, col: (WIDTH as i32 / 2) - 2, row: 0 }
    }

    fn mask(&self) -> u16 {
        TETROMINOES[self.kind].rotations[self.rotation]
    }

    fn color(&self) -> u8 {
        TETROMINOES[self.kind].color
    }

    fn blocks(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        let mask = self.mask();
        (0..16).filter(move |i| mask & (0x8000 >> i) != 0).map(move |i| {
            let r = i / 4;
            let c = i % 4;
            (self.col + c as i32, self.row + r as i32)
        })
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

fn render(board: &Board, piece: &Piece, score: usize, lines: usize) {
    print!("\x1B[H");
    println!("\x1B[97mTETRIS\x1B[m  Score: {:<6}  Lines: {:<4}  WASD=move  Q=quit", score, lines);

    print!("+");
    for _ in 0..WIDTH {
        print!("--");
    }
    println!("+");

    let piece_color = piece.color();
    for row in 0..HEIGHT {
        print!("|");
        for col in 0..WIDTH {
            let on_piece = piece.blocks().any(|(pc, pr)| pc == col as i32 && pr == row as i32);
            let color_index = if on_piece { piece_color } else { board.cells[row][col] };
            print!("{}\x1B[m", cell_color(color_index));
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
    let piece = Piece::new(0);
    render(&board, &piece, 0, 0);

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
