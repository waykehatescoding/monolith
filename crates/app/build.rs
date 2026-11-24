fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=../../assets/brand/icon.ico");
    if let Err(error) = embed_resource::compile("app.rc", embed_resource::NONE).manifest_optional()
    {
        panic!("failed to embed app resources: {error}");
    }
}
