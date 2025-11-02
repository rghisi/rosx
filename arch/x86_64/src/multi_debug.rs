use kernel::default_output::KernelOutput;

/// Fan-out debug writer that writes to multiple outputs
pub struct MultiDebugOutput {
    outputs: &'static [&'static dyn KernelOutput],
}

impl MultiDebugOutput {
    pub const fn new(outputs: &'static [&'static dyn KernelOutput]) -> Self {
        Self { outputs }
    }
}

impl KernelOutput for MultiDebugOutput {
    fn write_str(&self, s: &str) {
        for output in self.outputs {
            output.write_str(s);
        }
    }
}
