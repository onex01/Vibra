pub mod pci;
pub mod ahci;
pub mod usb;
pub mod virtio_gpu;

pub fn init() {
    pci::init();
    ahci::init();
    usb::init();
    virtio_gpu::init();
}
