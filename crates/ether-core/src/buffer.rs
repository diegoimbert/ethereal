//! Audio buffers passed to nodes: planar (one slice per channel), `f32`.

/// Input and output channels of one node for one (sub-)block. All slices have exactly
/// `ProcessContext::frames` samples. Inputs and outputs never alias; nodes that want to
/// process "in place" copy input to output first (the engine may elide that later).
pub struct AudioBuffers<'a, 'b> {
    pub inputs: &'a [&'b [f32]],
    pub outputs: &'a mut [&'b mut [f32]],
}

impl AudioBuffers<'_, '_> {
    /// Zero all outputs.
    pub fn clear_outputs(&mut self) {
        for ch in self.outputs.iter_mut() {
            ch.fill(0.0);
        }
    }

    /// Copy inputs to outputs channel-wise (extra outputs cleared). Handy for bypass.
    pub fn pass_through(&mut self) {
        for (i, out) in self.outputs.iter_mut().enumerate() {
            match self.inputs.get(i) {
                Some(input) => out.copy_from_slice(input),
                None => out.fill(0.0),
            }
        }
    }
}
