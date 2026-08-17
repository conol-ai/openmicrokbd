fn main() {
    if openmicro_app::status_ipc::run_cli_if_requested() {
        return;
    }
    openmicro_app::gpui_app::run()
}
