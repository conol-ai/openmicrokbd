fn main() {
    // `link.x` includes memory.x. Keep our flash/RAM map (bootloader pages,
    // info block placement, the carved-out vector-table copy and handoff
    // words) ahead of dependency search paths.
    println!("cargo:rustc-link-search={}", env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rerun-if-changed=memory.x");
}
