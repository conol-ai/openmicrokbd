fn main() {
    // `link.x` includes memory.x. Keep our board-specific memory map ahead of
    // dependency search paths so the final 2 KiB flash page is never linked
    // into the application image.
    println!("cargo:rustc-link-search={}", env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rerun-if-changed=memory.x");
}
