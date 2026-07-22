pub mod pci;
pub mod ahci;
pub mod usb;

pub fn init() {
    pci::init();
    ahci::init();
    usb::init();
}
