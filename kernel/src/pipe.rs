use syscall::wait;
use system::file::File;

pub struct Pipe {
    c: Option<char>
}

impl Pipe {
    pub fn new() -> Pipe {
        Pipe { c: None }
    }
}

impl File for Pipe {
    fn read_char(&self) -> char {
        while (!self.c.is_none()) {
            // wait();
        }

        self.c.unwrap()
    }

    fn write_char(&mut self, c: char) {
        self.c = Some(c);
    }
}