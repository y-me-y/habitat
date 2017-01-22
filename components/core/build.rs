extern crate gcc;
extern crate serde_codegen;

use std::env;
use std::path::Path;

#[cfg(windows)]
fn main() {
    codegen();
    gcc::compile_library("libadmincheck.a", &["./src/os/users/admincheck.c"]);
}

#[cfg(not(windows))]
fn main() {
    codegen();
}

fn codegen() {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let src = Path::new("src/serde_types.in.rs");
    let dst = Path::new(&out_dir).join("serde_types.rs");
    serde_codegen::expand(&src, &dst).unwrap();
}
