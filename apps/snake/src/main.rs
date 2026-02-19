#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::VecDeque;
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

const WIDTH: usize = 20;
const HEIGHT: usize = 18;
const FRAME_MS: u64 = 150;

#[derive(Clone, Copy, PartialEq)]
struct Pos {
    x: usize,
    y: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }
}

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

fn random_food(snake: &VecDeque<Pos>, rng: &mut Rng) -> Pos {
    loop {
        let pos = Pos {
            x: rng.next_usize(WIDTH),
            y: rng.next_usize(HEIGHT),
        };
        if !snake.iter().any(|&s| s == pos) {
            return pos;
        }
    }
}

fn advance(pos: Pos, dir: Dir) -> Option<Pos> {
    match dir {
        Dir::Up => {
            if pos.y == 0 { None } else { Some(Pos { x: pos.x, y: pos.y - 1 }) }
        }
        Dir::Down => {
            if pos.y + 1 >= HEIGHT { None } else { Some(Pos { x: pos.x, y: pos.y + 1 }) }
        }
        Dir::Left => {
            if pos.x == 0 { None } else { Some(Pos { x: pos.x - 1, y: pos.y }) }
        }
        Dir::Right => {
            if pos.x + 1 >= WIDTH { None } else { Some(Pos { x: pos.x + 1, y: pos.y }) }
        }
    }
}

fn render(snake: &VecDeque<Pos>, food: Pos, score: usize, game_over: bool) {
    print!("\x1B[H");

    if game_over {
        print!("\x1B[97mSNAKE  \x1B[91mGAME OVER!\x1B[m  Score: {:<4}  R=restart Q=quit   \n", score);
    } else {
        print!("\x1B[97mSNAKE\x1B[m  Score: {:<4}  WASD=move  Q=quit          \n", score);
    }

    print!("+");
    for _ in 0..WIDTH {
        print!("-");
    }
    println!("+");

    let head = snake.back().copied();

    for row in 0..HEIGHT {
        print!("|");
        for col in 0..WIDTH {
            let pos = Pos { x: col, y: row };
            if head == Some(pos) {
                print!("\x1B[102m \x1B[m");
            } else if snake.iter().any(|&s| s == pos) {
                print!("\x1B[42m \x1B[m");
            } else if pos == food {
                print!("\x1B[41m \x1B[m");
            } else {
                print!(" ");
            }
        }
        println!("|");
    }

    print!("+");
    for _ in 0..WIDTH {
        print!("-");
    }
    println!("+");
}

fn play(rng: &mut Rng) -> bool {
    let mut snake: VecDeque<Pos> = VecDeque::new();
    let cx = WIDTH / 2;
    let cy = HEIGHT / 2;
    snake.push_back(Pos { x: cx - 2, y: cy });
    snake.push_back(Pos { x: cx - 1, y: cy });
    snake.push_back(Pos { x: cx, y: cy });

    let mut food = random_food(&snake, rng);
    let mut dir = Dir::Right;
    let mut score = 0usize;

    loop {
        render(&snake, food, score, false);
        Syscall::sleep(FRAME_MS);

        while let Some(c) = Syscall::try_read_char() {
            let new_dir = match c {
                'w' | 'W' => Some(Dir::Up),
                's' | 'S' => Some(Dir::Down),
                'a' | 'A' => Some(Dir::Left),
                'd' | 'D' => Some(Dir::Right),
                'q' | 'Q' => {
                    render(&snake, food, score, true);
                    return true;
                }
                _ => None,
            };
            if let Some(d) = new_dir {
                if d != dir.opposite() {
                    dir = d;
                }
            }
        }

        let head = *snake.back().unwrap();
        let new_head = match advance(head, dir) {
            Some(p) => p,
            None => {
                render(&snake, food, score, true);
                return false;
            }
        };

        if snake.iter().any(|&s| s == new_head) {
            render(&snake, food, score, true);
            return false;
        }

        snake.push_back(new_head);
        if new_head == food {
            score += 1;
            food = random_food(&snake, rng);
        } else {
            snake.pop_front();
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

        loop {
            let c = Syscall::read_char();
            match c {
                'r' | 'R' => break,
                'q' | 'Q' => {
                    println!("Goodbye!");
                    return;
                }
                _ => {}
            }
        }

        print!("\x1B[2J\x1B[H");
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
