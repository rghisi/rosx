use core::fmt::{self, Write};

static mut DEFAULT_OUTPUT: Option<&'static dyn KernelOutput> = None;

struct KernelWriter;

impl Write for KernelWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        unsafe {
            if let Some(output) = DEFAULT_OUTPUT {
                output.write_str(s);
                Ok(())
            } else {
                Err(fmt::Error)
            }
        }
    }
}

pub trait KernelOutput: Send + Sync {
    fn write_str(&self, s: &str);
}
pub(crate) fn setup_default_output(output: &'static dyn KernelOutput) {
    unsafe {
        DEFAULT_OUTPUT = Some(output);
    }
}

pub struct MultiplexOutput {
    outputs: &'static [&'static dyn KernelOutput],
}

impl MultiplexOutput {
    pub const fn new(outputs: &'static [&'static dyn KernelOutput]) -> Self {
        Self { outputs }
    }
}

impl KernelOutput for MultiplexOutput {
    fn write_str(&self, s: &str) {
        for output in self.outputs {
            output.write_str(s);
        }
    }
}

#[doc(hidden)]
pub fn print(args: fmt::Arguments) {
    use core::fmt::Write;
    KernelWriter.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! kprintln {
      () => ($crate::kprint!("\n"));
      ($($arg:tt)*) => ($crate::kprint!("{}\n", format_args!($($arg)*)));
  }

#[macro_export]
macro_rules! kprint {
      ($($arg:tt)*) => ($crate::default_output::print(format_args!($($arg)*)));
  }
