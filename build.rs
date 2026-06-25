// build.rs - Generate CAEN FELib and CAENDigitizer bindings using bindgen

use std::env;
use std::path::PathBuf;

fn main() {
    // CAEN FELib / Digitizer install prefix. Defaults to the standard
    // `/usr/local`, but can be overridden with the `CAEN_PREFIX` env var to
    // point at an isolated install (e.g. `/opt/delila-caen`). This is what
    // lets delila-rs ship its own CAEN stack on a shared machine without
    // touching the system libs other software (CoMPASS) depends on — see
    // `scripts/setup_caen_felib.sh`.
    let prefix = env::var("CAEN_PREFIX").unwrap_or_else(|_| "/usr/local".to_string());
    let lib_dir = format!("{prefix}/lib");
    let include_dir = format!("{prefix}/include");
    let inc_arg = format!("-I{include_dir}");

    // Tell cargo where to find the CAEN shared libraries.
    println!("cargo:rustc-link-search={lib_dir}");

    // Tell cargo to link the CAEN_FELib library
    println!("cargo:rustc-link-lib=CAEN_FELib");

    // Link CAENDigitizer library for x743 support
    #[cfg(feature = "x743")]
    {
        println!("cargo:rustc-link-lib=CAENDigitizer");
    }

    // CAEN_FELib dlopen()s its dig1/dig2 backends and calls dlerror(); on
    // glibc < 2.34 (e.g. Ubuntu 20.04) those symbols live in libdl, so link it
    // explicitly or the binary dies with "undefined symbol: dlerror" at the
    // first FELib call. Harmless on newer glibc (merged into libc). Linux-only:
    // macOS provides them via libSystem.
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-lib=dl");

    // Bake an rpath so the binaries resolve the prefix's CAEN libs at runtime
    // without needing LD_LIBRARY_PATH — essential when `prefix` is not a
    // ldconfig search path (e.g. /opt/delila-caen). Harmless for the default
    // /usr/local (already a system path).
    println!("cargo:rustc-link-arg=-Wl,-rpath,{lib_dir}");

    // Rebuild if the prefix or the C wrapper changes.
    println!("cargo:rerun-if-env-changed=CAEN_PREFIX");
    println!("cargo:rerun-if-changed=src/reader/caen/wrapper.h");
    println!("cargo:rerun-if-changed=src/reader/caen/wrapper.c");

    // Compile C wrapper for variadic functions
    // Rust cannot directly call C variadic functions on all platforms (especially macOS ARM64)
    cc::Build::new()
        .file("src/reader/caen/wrapper.c")
        .include(&include_dir)
        .compile("caen_wrapper");

    // Generate FELib bindings
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    let felib_bindings = bindgen::Builder::default()
        .header("src/reader/caen/wrapper.h")
        .clang_arg(&inc_arg)
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

        let digitizer_bindings = bindgen::Builder::default()
            .header("src/reader/caen_legacy/wrapper.h")
            // Resolve CAENDigitizer.h from the system install (same model as FELib above),
            // not a vendored copy — keeps the repo free of CAEN's GPL-licensed headers.
            .clang_arg(&inc_arg)
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
