fn main() {
    assert_eq!(
        std::env::var("CARGO_CFG_TARGET_OS").unwrap(),
        "linux",
        "Linux backend requires Linux"
    );
    let mut build = cc::Build::new();
    for (library, version) in [
        ("libimobiledevice-1.0", "1.3.0"),
        ("libplist-2.0", "2.2.0"),
        ("libusbmuxd-2.0", "2.0.0"),
    ] {
        let lib = pkg_config::Config::new()
            .atleast_version(version)
            .probe(library)
            .unwrap_or_else(|_| {
                panic!("Missing {library} >= {version}; see README native development dependencies")
            });
        for path in lib.include_paths {
            build.include(path);
        }
    }
    build
        .file("src/adapter.c")
        .warnings(true)
        .warnings_into_errors(true)
        .compile("aircard_linux");
    println!("cargo:rerun-if-changed=src/adapter.c");
}
