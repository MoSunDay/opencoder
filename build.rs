fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    // Debug TUI futures exceed MSVC's default 1 MiB executable stack.
    // Reserve address space; Windows commits stack pages only as needed.
    if target.ends_with("windows-msvc") {
        println!("cargo:rustc-link-arg-bin=opencoder=/STACK:8388608");
    } else if target.ends_with("windows-gnu") {
        println!("cargo:rustc-link-arg-bin=opencoder=-Wl,--stack,8388608");
    }
}
