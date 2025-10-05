use core::fmt::{self, Write};

pub trait DebugOutput: Send + Sync {
    fn write_str(&self, s: &str);
}

static mut DEBUG_OUTPUT: Option<&'static dyn DebugOutput> = None;

pub fn init_debug(output: &'static dyn DebugOutput) {
    unsafe {
        DEBUG_OUTPUT = Some(output);
    }
}

struct DebugWriter;

impl fmt::Write for DebugWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        unsafe {
            if let Some(output) = DEBUG_OUTPUT {
                output.write_str(s);
                Ok(())
            } else {
                Err(fmt::Error)
            }
        }
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    DebugWriter.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! kprintln {
      () => ($crate::kprint!("\n"));
      ($($arg:tt)*) => ($crate::kprint!("{}\n", format_args!($($arg)*)));
  }

#[macro_export]
macro_rules! kprint {
      ($($arg:tt)*) => ($crate::debug::_print(format_args!($($arg)*)));
  }
