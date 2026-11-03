use std::process::Command;

fn main() {
    let output = Command::new("brew")
        .args(["--prefix", "mupdf"])
        .output()
        .expect("tpdf needs Homebrew to locate mupdf");
    let stdout = String::from_utf8(output.stdout).expect("brew printed invalid UTF-8");
    let prefix = stdout.trim();

    cc::Build::new()
        .file("src/shim.c")
        .include(format!("{prefix}/include"))
        .opt_level(3)
        .compile("shim");

    println!("cargo:rustc-link-search=native={prefix}/lib");
    println!("cargo:rustc-link-lib=dylib=mupdf");
    println!("cargo:rerun-if-changed=src/shim.c");
}
