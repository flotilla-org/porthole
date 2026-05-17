fn main() {
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rerun-if-changed=src/commands/record_av_writer_shim.m");
        cc::Build::new()
            .file("src/commands/record_av_writer_shim.m")
            .flag("-fobjc-arc")
            .compile("porthole_record_av_writer");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rustc-link-lib=framework=CoreVideo");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }
}
