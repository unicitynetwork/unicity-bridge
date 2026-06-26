#[test]
fn bridge_vectors_match_core_derivations() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("bridge-vectors");
    bridge_return_host::check_vectors(&root).expect("bridge vectors match");
}
