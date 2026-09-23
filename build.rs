use std::fs;

use yaml_rust2::YamlLoader;

const SPEC_PATH: &str = "spec/api.yaml";

fn main() {
    println!("cargo:rerun-if-changed={SPEC_PATH}");
    let source = fs::read_to_string(SPEC_PATH).unwrap_or_else(|e| panic!("{SPEC_PATH}: {e}"));
    let docs = YamlLoader::load_from_str(&source).unwrap_or_else(|e| panic!("{SPEC_PATH}: {e}"));
    let version = docs[0]["info"]["version"]
        .as_str()
        .expect("spec: нет info.version");
    println!("cargo:rustc-env=APLAUT_SPEC_VERSION={version}");
}
