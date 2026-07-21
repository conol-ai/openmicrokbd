// Stub main: some platforms build as a dylib (wasm/mobile), so the app
// itself lives in the library crate (makepad convention).
fn main() {
    openmicro_app::app::app_main()
}
