fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=native/ffmpeg.cpp");
    for lib in ["d3d11", "dxgi", "advapi32"] {
        println!("cargo:rustc-link-lib={lib}");
    }
    println!("cargo:rerun-if-changed=../../vendor/ffmpeg/libavutil/avconfig.h");
    cc::Build::new()
        .cpp(true)
        .file("native/ffmpeg.cpp")
        .include("../../vendor/ffmpeg")
        .flag_if_supported("/std:c++17")
        .flag_if_supported("/EHsc")
        .warnings(false)
        .compile("p2pshare_ffmpeg");
}
