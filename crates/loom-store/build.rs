fn main() {
    // Stable Rust tracks files already expanded by migrate!(), but not a newly
    // added migration. Watch the directory so cached builds embed additions
    // and removals too: https://docs.rs/sqlx/0.8.6/sqlx/macro.migrate.html
    println!("cargo:rerun-if-changed=migrations");
}
