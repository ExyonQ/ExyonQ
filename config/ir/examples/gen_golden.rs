use exyonq_config_ir::{fingerprint, AppConfig};
use std::collections::HashMap;
use std::path::PathBuf;

fn main() {
    let fixtures = ["minimal.toml", "static.toml", "modules.toml"];
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut map = HashMap::new();
    for name in fixtures {
        let path = base.join(name);
        let config = AppConfig::from_file(&path).expect("fixture");
        map.insert(name.to_string(), fingerprint(&config).as_str().to_string());
    }
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/golden/ir-fingerprints.json");
    std::fs::create_dir_all(out.parent().unwrap()).ok();
    std::fs::write(&out, serde_json::to_string_pretty(&map).unwrap()).unwrap();
    println!("wrote {}", out.display());
}
