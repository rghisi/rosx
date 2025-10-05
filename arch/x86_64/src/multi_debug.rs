use kernel::debug::DebugOutput;

/// Fan-out debug writer that writes to multiple outputs
pub struct MultiDebugOutput {
    outputs: &'static [&'static dyn DebugOutput],
}

impl MultiDebugOutput {
    pub const fn new(outputs: &'static [&'static dyn DebugOutput]) -> Self {
        Self { outputs }
    }
}

impl DebugOutput for MultiDebugOutput {
    fn write_str(&self, s: &str) {
        for output in self.outputs {
            output.write_str(s);
        }
    }
}
