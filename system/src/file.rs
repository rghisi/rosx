pub trait File {
    fn read_char(&self) -> char;

    fn write_char(&mut self, c: char);
}