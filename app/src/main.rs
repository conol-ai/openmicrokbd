#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    if openmicro_app::status_ipc::run_cli_if_requested() {
        return;
    }
    openmicro_app::gpui_app::run()
}
