// build.rs - Generate CAEN FELib and CAENDigitizer bindings using bindgen

use std::env;
use std::path::PathBuf;

fn main() {
    // Tell cargo to look for shared libraries in /usr/local/lib
    println!("cargo:rustc-link-search=/usr/local/lib");

    // Tell cargo to link the CAEN_FELib library
    println!("cargo:rustc-link-lib=CAEN_FELib");

    // Link CAENDigitizer library for x743 support
    #[cfg(feature = "x743")]
    {
        println!("cargo:rustc-link-lib=CAENDigitizer");
    }

    // macOS: Set rpath for runtime library loading
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/local/lib");

    // Tell cargo to invalidate the built crate whenever the wrapper changes
    println!("cargo:rerun-if-changed=src/reader/caen/wrapper.h");
    println!("cargo:rerun-if-changed=src/reader/caen/wrapper.c");

    // Compile C wrapper for variadic functions
    // Rust cannot directly call C variadic functions on all platforms (especially macOS ARM64)
    cc::Build::new()
        .file("src/reader/caen/wrapper.c")
        .include("/usr/local/include")
        .compile("caen_wrapper");

    // Generate FELib bindings
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    let felib_bindings = bindgen::Builder::default()
        .header("src/reader/caen/wrapper.h")
        .clang_arg("-I/usr/local/include")
        .allowlist_function("CAEN_FELib_.*")
        .allowlist_type("CAEN_FELib_.*")
        .allowlist_var("CAEN_FELIB_.*")
        .rustified_enum("CAEN_FELib_ErrorCode")
        .generate_comments(true)
        .derive_debug(true)
        .derive_default(true)
        .derive_eq(true)
        .derive_hash(true)
        .generate()
        .expect("Unable to generate FELib bindings");

    felib_bindings
        .write_to_file(out_path.join("caen_felib_bindings.rs"))
        .expect("Couldn't write FELib bindings!");

    // Generate CAENDigitizer bindings for x743 support
    #[cfg(feature = "x743")]
    {
        println!("cargo:rerun-if-changed=src/reader/caen_legacy/wrapper.h");
        println!("cargo:rerun-if-changed=src/reader/caen_legacy/CAENDigitizer.h");
        println!("cargo:rerun-if-changed=src/reader/caen_legacy/CAENDigitizerType.h");

        let digitizer_bindings = bindgen::Builder::default()
            .header("src/reader/caen_legacy/wrapper.h")
            .clang_arg("-Isrc/reader/caen_legacy")
            .allowlist_function("CAEN_DGTZ_.*")
            .allowlist_type("CAEN_DGTZ_.*")
            .allowlist_var("CAEN_DGTZ_.*")
            .allowlist_var("MAX_V1743_GROUP_SIZE")
            .allowlist_var("MAX_X743_CHANNELS_X_GROUP")
            // Use Rust enums for key C enums
            .rustified_enum("CAEN_DGTZ_ErrorCode")
            .rustified_enum("CAEN_DGTZ_ConnectionType")
            .rustified_enum("CAEN_DGTZ_AcqMode_t")
            .rustified_enum("CAEN_DGTZ_ReadMode_t")
            .rustified_enum("CAEN_DGTZ_TriggerMode_t")
            .rustified_enum("CAEN_DGTZ_TriggerPolarity_t")
            .rustified_enum("CAEN_DGTZ_IOLevel_t")
            .rustified_enum("CAEN_DGTZ_SAMFrequency_t")
            .rustified_enum("CAEN_DGTZ_SAM_CORRECTION_LEVEL_t")
            .rustified_enum("CAEN_DGTZ_SAMPulseSourceType_t")
            .rustified_enum("CAEN_DGTZ_AcquisitionMode_t")
            .rustified_enum("CAEN_DGTZ_TrigerLogic_t")
            .rustified_enum("CAEN_DGTZ_EnaDis_t")
            .generate_comments(true)
            .derive_debug(true)
            .derive_default(true)
            .generate()
            .expect("Unable to generate CAENDigitizer bindings");

        digitizer_bindings
            .write_to_file(out_path.join("caen_digitizer_bindings.rs"))
            .expect("Couldn't write CAENDigitizer bindings!");
    }
}
