// lsusb — показать найденные USB контроллеры.

use vibra_kernel::commands::CmdResult;
use vibra_kernel::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW};
use alloc::format;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let controllers = vibra_kernel::drivers::usb::get_controllers();

    if controllers.is_empty() {
        console.print_colored("USB контроллеры не найдены\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    console.print_colored("Найдено USB контроллеров: ", COLOR_CYAN);
    console.print_num(controllers.len());
    console.print("\n\n");

    for (i, ctrl) in controllers.iter().enumerate() {
        let version_major = (ctrl.version >> 8) as u8;
        let version_minor = (ctrl.version & 0xFF) as u8;

        console.print_colored(&format!("  USB{}: {:04X}:{:04X} xHCI v{}.{} ({} портов)\n",
            i, ctrl.vendor, ctrl.device,
            version_major, version_minor,
            ctrl.port_count), COLOR_YELLOW);
    }

    CmdResult::Ok
}
