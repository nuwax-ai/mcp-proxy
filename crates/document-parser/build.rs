fn main() {
    println!("cargo:rerun-if-changed=locales/en.yml");
    println!("cargo:rerun-if-changed=locales/zh-CN.yml");
    println!("cargo:rerun-if-changed=locales/zh-TW.yml");
}
