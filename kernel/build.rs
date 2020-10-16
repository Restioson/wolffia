use std::env;
fn main() {
    let path = format!("../../build/{}/init.elf", env::var("PROFILE").unwrap());
    println!("cargo:rustc-env=WOLFFIA_INIT_PATH={}", path);
}
