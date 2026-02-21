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

struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_bool(&mut self, threshold: u64) -> bool {
        self.state = self.state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 33) % 100 < threshold
    }
}

fn randomize(grid: &mut Grid, rng: &mut Rng) {
    for row in 0..ROWS {
        for col in 0..COLS {
            grid.cells[row][col] = rng.next_bool(30);
        }
    }
}

fn neighbours(grid: &Grid, row: usize, col: usize) -> u8 {
    let mut count = 0;
    for dr in [ROWS - 1, 0, 1] {
        for dc in [COLS - 1, 0, 1] {
            if dr == 0 && dc == 0 {
                continue;
            }
            let r = (row + dr) % ROWS;
            let c = (col + dc) % COLS;
            if grid.cells[r][c] {
                count += 1;
            }
        }
    }
    count
}

fn step(current: &Grid, next: &mut Grid) -> usize {
    let mut population = 0;
    for row in 0..ROWS {
        for col in 0..COLS {
            let n = neighbours(current, row, col);
            let alive = match (current.cells[row][col], n) {
                (true, 2) | (true, 3) => true,
                (false, 3) => true,
                _ => false,
            };
            next.cells[row][col] = alive;
            if alive {
                population += 1;
            }
        }
    }
    population
}

fn render(grid: &Grid, generation: usize, population: usize, delay_ms: u64, paused: bool) {
    print!("\x1B[H");

    if paused {
        println!("\x1B[97mCONWAY\x1B[m Gen: {:<6} Pop: {:<4} {:<4}ms \x1B[93mPAUSED\x1B[m Spc=pause Q=quit",
            generation, population, delay_ms);
    } else {
        println!("\x1B[97mCONWAY\x1B[m Gen: {:<6} Pop: {:<4} {:<4}ms Spc=pause =/-=speed R=rand Q=quit",
            generation, population, delay_ms);
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

    let mut rng = Rng::new(0xDEAD_BEEF_CAFE_BABE);
    let mut current = Grid::new();
    let mut next = Grid::new();
    let mut generation = 0usize;
    let mut population = 0usize;
    let mut paused = false;
    let mut delay_ms: u64 = 100;

    randomize(&mut current, &mut rng);

    loop {
        render(&current, generation, population, delay_ms, paused);
        if delay_ms > 0 {
            Syscall::sleep(delay_ms);
        }

        while let Some(c) = Syscall::try_read_char() {
            match c {
                ' ' => paused = !paused,
                '+' | '=' => delay_ms = delay_ms.saturating_sub(50),
                '-' => delay_ms = (delay_ms + 50).min(1000),
                'r' | 'R' => {
                    randomize(&mut current, &mut rng);
                    generation = 0;
                    population = 0;
                }
                'q' | 'Q' => return,
                _ => {}
            }
        }

        if !paused {
            population = step(&current, &mut next);
            core::mem::swap(&mut current, &mut next);
            generation += 1;
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
