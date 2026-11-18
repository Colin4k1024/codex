//! Export JSON schemas for the product contracts (P2-1 / #78).
//! Usage: cargo run --bin export-schemas -- <out-dir>

use std::path::PathBuf;

use superagent_contracts::{ArtifactSpec, Gate, Node, Workflow};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "schema/json".to_string());
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("create schema dir");
    let entries: Vec<(&str, schemars::schema::RootSchema)> = vec![
        ("Workflow", schemars::schema_for!(Workflow)),
        ("Node", schemars::schema_for!(Node)),
        ("Gate", schemars::schema_for!(Gate)),
        ("ArtifactSpec", schemars::schema_for!(ArtifactSpec)),
    ];
    for (name, schema) in entries {
        let path = out.join(format!("{name}.json"));
        let text = serde_json::to_string_pretty(&schema).expect("serialize schema") + "\n";
        std::fs::write(&path, text).expect("write schema");
        println!("wrote {}", path.display());
    }
}