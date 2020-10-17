use std::env;

fn main() {
    let profile = env::var("PROFILE").unwrap();
    let path = format!("../../build/{}/init.elf", profile);
    println!("cargo:rustc-env=WOLFFIA_INIT_PATH={}", path);
}
