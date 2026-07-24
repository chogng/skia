use skia_gpu::{GpuBackend, GpuCommandBuffer};

/// Records synchronous backend submissions completed by one test.
///
/// The current backend contract waits for each submission before returning, so
/// successful submissions are immediately finished. Keeping that state in one
/// object mirrors upstream flush-finish tracking without inventing asynchronous
/// callbacks that the portable API does not expose yet.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct SubmissionTracker {
    submitted: u64,
    finished: u64,
}

impl SubmissionTracker {
    /// Returns the number of submission attempts recorded by this tracker.
    pub const fn submitted(self) -> u64 {
        self.submitted
    }

    /// Returns the number of successfully completed submissions.
    pub const fn finished(self) -> u64 {
        self.finished
    }

    /// Returns whether every tracked submission has finished.
    pub const fn is_finished(self) -> bool {
        self.submitted == self.finished
    }

    /// Submits one command buffer and records completion when it succeeds.
    pub fn submit<B: GpuBackend>(
        &mut self,
        backend: &mut B,
        surface: &mut B::Surface,
        commands: &GpuCommandBuffer,
    ) -> Result<(), B::Error> {
        self.submitted = self.submitted.saturating_add(1);
        backend.submit(surface, commands)?;
        self.finished = self.finished.saturating_add(1);
        Ok(())
    }
}
