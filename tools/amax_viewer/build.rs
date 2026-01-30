// build.rs - Set library paths for CAEN FELib

fn main() {
    // Tell cargo to look for shared libraries in /usr/local/lib
    println!("cargo:rustc-link-search=/usr/local/lib");

    // macOS: Set rpath for runtime library loading
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/local/lib");
}
