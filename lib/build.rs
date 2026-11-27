fn main() {
    println!(
        "cargo:rustc-env=GA4GH_EXTENSION_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
