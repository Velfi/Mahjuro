fn main() {
    if std::env::var("CARGO_FEATURE_WINDOWS_STORE").is_ok() {
        let mut build = cc::Build::new();
        build
            .file("cpp/xbox_shim/xbox_shim.cpp")
            .cpp(true)
            .std("c++17");
        build.compile("xbox_shim");
        println!("cargo:rerun-if-changed=cpp/xbox_shim/xbox_shim.cpp");
        println!("cargo:rerun-if-changed=cpp/xbox_shim/xbox_shim.h");
    }
}
