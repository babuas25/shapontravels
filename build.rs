fn main() {
    // sqlx::migrate! tracks existing files, but stable Rust also needs this to
    // rebuild the embedded migrator when a new migration file is added.
    println!("cargo:rerun-if-changed=migrations");
}
