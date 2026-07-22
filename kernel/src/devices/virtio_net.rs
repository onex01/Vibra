// VirtIO Network — заглушка для сетевого устройства.
// Реализация VRing + DMA будет добавлена позже.

pub struct VirtioNet {
    base: u64,
    ready: bool,
}

impl VirtioNet {
    pub fn new(base: u64) -> Self {
        Self { base, ready: false }
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }
}
