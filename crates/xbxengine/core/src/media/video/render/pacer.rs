#[derive(Clone, Debug)]
pub struct DecodedFrame;

pub struct FramePacer;

impl FramePacer {
    pub fn submit(&mut self, _frame: DecodedFrame) {}

    pub fn next_frame(&mut self) -> Option<DecodedFrame> {
        None
    }
}
