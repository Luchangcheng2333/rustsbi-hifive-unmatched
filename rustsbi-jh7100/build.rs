// 添加链接器脚本

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-link-arg=-Trustsbi-jh7100/src/u740.ld");
}
