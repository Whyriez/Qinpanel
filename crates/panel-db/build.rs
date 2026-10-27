// SQLx embeds migrations in the library. Additions must invalidate its build.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=operations_migrations");
}
