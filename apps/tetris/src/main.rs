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
const FRAME_MS: u64 = 500;

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

struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_usize(&mut self, max: usize) -> usize {
        self.state = self.state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 33) as usize % max
    }
}

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

    fn color(&self) -> u8 {
        TETROMINOES[self.kind].color
    }

    fn blocks_at(&self, col: i32, row: i32, rotation: usize) -> impl Iterator<Item = (i32, i32)> {
        let mask = TETROMINOES[self.kind].rotations[rotation];
        (0..16u32).filter(move |i| mask & (0x8000u16 >> i) != 0).map(move |i| {
            let r = i / 4;
            let c = i % 4;
            (col + c as i32, row + r as i32)
        })
    }

    fn blocks(&self) -> impl Iterator<Item = (i32, i32)> {
        self.blocks_at(self.col, self.row, self.rotation)
    }
}

fn collides(board: &Board, piece: &Piece, col: i32, row: i32, rotation: usize) -> bool {
    for (c, r) in piece.blocks_at(col, row, rotation) {
        if c < 0 || c >= WIDTH as i32 || r >= HEIGHT as i32 {
            return true;
        }
        if r >= 0 && board.cells[r as usize][c as usize] != 0 {
            return true;
        }
    }
    false
}

fn lock(board: &mut Board, piece: &Piece) {
    for (c, r) in piece.blocks() {
        if r >= 0 && r < HEIGHT as i32 && c >= 0 && c < WIDTH as i32 {
            board.cells[r as usize][c as usize] = piece.color();
        }
    }
}

fn clear_lines(board: &mut Board) -> usize {
    let mut cleared = 0;
    let mut row = HEIGHT as i32 - 1;
    while row >= 0 {
        if board.cells[row as usize].iter().all(|&c| c != 0) {
            for r in (1..=row as usize).rev() {
                board.cells[r] = board.cells[r - 1];
            }
            board.cells[0] = [0; WIDTH];
            cleared += 1;
        } else {
            row -= 1;
        }
    }
    cleared
}

fn line_score(cleared: usize) -> usize {
    match cleared {
        1 => 100,
        2 => 300,
        3 => 500,
        4 => 800,
        _ => 0,
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

fn render_next(next_kind: usize, row: usize) {
    let mask = TETROMINOES[next_kind].rotations[0];
    let color = TETROMINOES[next_kind].color;
    for r in 0..4 {
        for c in 0..4 {
            let bit = 0x8000u16 >> (r * 4 + c);
            if mask & bit != 0 {
                print!("{}\x1B[m ", cell_color(color));
            } else {
                print!("  ");
            }
        }
        if r < 3 {
            print!("\x1B[1A\x1B[9C");
        }
        let _ = row;
    }
}

fn render(board: &Board, piece: &Piece, next_kind: usize, score: usize, lines: usize, level: usize) {
    print!("\x1B[H");
    println!("\x1B[97mTETRIS\x1B[m  Score: {:<6}  Lines: {:<4}  Level: {:<3}  WASD=move  Q=quit", score, lines, level);

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
        print!("|");

        match row {
            1 => print!("  NEXT:"),
            3 => { print!("  "); render_next(next_kind, row); }
            _ => {}
        }

        println!();
    }

    print!("+");
    for _ in 0..WIDTH {
        print!("--");
    }
    println!("+");
}

fn play(rng: &mut Rng) -> bool {
    let mut board = Board::new();
    let mut piece = Piece::new(rng.next_usize(7));
    let mut next_kind = rng.next_usize(7);
    let mut score = 0usize;
    let mut lines = 0usize;
    let mut frame_ms = FRAME_MS;

    loop {
        let level = lines / 10 + 1;
        render(&board, &piece, next_kind, score, lines, level);
        Syscall::sleep(frame_ms);

        while let Some(c) = Syscall::try_read_char() {
            match c {
                'a' | 'A' => {
                    if !collides(&board, &piece, piece.col - 1, piece.row, piece.rotation) {
                        piece.col -= 1;
                    }
                }
                'd' | 'D' => {
                    if !collides(&board, &piece, piece.col + 1, piece.row, piece.rotation) {
                        piece.col += 1;
                    }
                }
                'w' | 'W' => {
                    let new_rot = (piece.rotation + 1) % 4;
                    let kicks: [i32; 5] = [0, -1, 1, -2, 2];
                    for kick in kicks {
                        if !collides(&board, &piece, piece.col + kick, piece.row, new_rot) {
                            piece.col += kick;
                            piece.rotation = new_rot;
                            break;
                        }
                    }
                }
                's' | 'S' => {
                    if !collides(&board, &piece, piece.col, piece.row + 1, piece.rotation) {
                        piece.row += 1;
                        score += 1;
                    }
                }
                'q' | 'Q' => return true,
                _ => {}
            }
        }

        if collides(&board, &piece, piece.col, piece.row + 1, piece.rotation) {
            lock(&mut board, &piece);
            let cleared = clear_lines(&mut board);
            lines += cleared;
            score += line_score(cleared);
            frame_ms = (FRAME_MS.saturating_sub((lines / 10) as u64 * 50)).max(100);
            piece = Piece::new(next_kind);
            next_kind = rng.next_usize(7);
        } else {
            piece.row += 1;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    let mut rng = Rng::new(0xDEAD_BEEF_CAFE_BABE);
    print!("\x1B[2J\x1B[H");

    loop {
        let quit = play(&mut rng);
        if quit {
            println!("Goodbye!");
            return;
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
