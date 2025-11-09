use core::fmt;

pub fn print(args: fmt::Arguments) {
    kernel::default_output::print(args);
}
#[macro_export]
macro_rules! println {
      () => ($crate::print!("\n"));
      ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
  }

#[macro_export]
macro_rules! print {
      ($($arg:tt)*) => ($crate::out::print(format_args!($($arg)*)));
  }
