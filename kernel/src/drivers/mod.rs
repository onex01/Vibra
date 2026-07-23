pub mod pci;
pub mod ahci;
pub mod usb;
pub mod virtio_gpu;
pub mod nvme;
pub mod e1000;
pub mod rtl8139;

pub fn init() {
    pci::init();
    ahci::init();
    nvme::init();
    usb::init();
    virtio_gpu::init();
    e1000::init();
    rtl8139::init();
}
