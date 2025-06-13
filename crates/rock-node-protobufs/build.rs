// File: rock-node-workspace/crates/rock-node-protobufs/build.rs
// (Final Idempotent Version)

use anyhow::Result;
use fs_extra::dir::{copy, CopyOptions};
use std::env;
use std::path::PathBuf;
use std::process::Command;
use walkdir::WalkDir;

fn main() -> Result<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.join("../..").canonicalize()?;
    let local_proto_root = workspace_root.join("proto");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // This is the single, temporary directory where we will assemble all .proto files.
    let unified_proto_dir = out_dir.join("unified_proto");

    // --- ** THIS IS THE FIX ** ---
    // Clean up the unified directory from the previous build, if it exists,
    // then create it fresh. This makes the script idempotent.
    if unified_proto_dir.exists() {
        std::fs::remove_dir_all(&unified_proto_dir)?;
    }
    std::fs::create_dir_all(&unified_proto_dir)?;
    // --- ** END FIX ** ---

    println!("cargo:rerun-if-changed={}", local_proto_root.display());

    copy(&local_proto_root, &unified_proto_dir, &CopyOptions::new().content_only(true))?;

    let temp_consensus_node_dir = out_dir.join("hiero-consensus-node");
    let consensus_node_proto_src = temp_consensus_node_dir.join("hapi/hedera-protobuf-java-api/src/main/proto");
    
    if !consensus_node_proto_src.exists() {
        println!("cargo:warning=Cloning hiero-consensus-node repository...");
        let repo_url = "https://github.com/hiero-ledger/hiero-consensus-node.git";
        let cn_tag_hash = "efb0134e921b32ed6302da9c93874d65492e876f"; 
        run_command(Command::new("git").arg("clone").arg("--depth=1").arg("--filter=blob:none").arg("--sparse").arg(repo_url).arg(&temp_consensus_node_dir))?;
        let sparse_checkout_path = "hapi/hedera-protobuf-java-api/src/main/proto";
        run_command(Command::new("git").current_dir(&temp_consensus_node_dir).arg("sparse-checkout").arg("set").arg(sparse_checkout_path))?;
        run_command(Command::new("git").current_dir(&temp_consensus_node_dir).arg("checkout").arg(cn_tag_hash))?;
    }
    
    let external_dirs_to_copy = ["block", "platform", "services", "streams", "mirror"];
    for dir in external_dirs_to_copy {
        let src_path = consensus_node_proto_src.join(dir);
        if src_path.exists() {
            copy(&src_path, &unified_proto_dir, &CopyOptions::new())?;
        }
    }

    let all_protos: Vec<PathBuf> = WalkDir::new(&unified_proto_dir)
        .into_iter().filter_map(Result::ok)
        .filter(|e| e.file_type().is_file() && e.path().extension().map_or(false, |ext| ext == "proto"))
        .map(|e| e.into_path())
        .collect();

    if all_protos.is_empty() {
        panic!("Build failed: No .proto files found after copy operations.");
    }
    
    println!("cargo:warning=Compiling {} protobuf definitions from unified directory...", all_protos.len());
    tonic_build::configure()
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .compile(&all_protos, &[unified_proto_dir])?;

    Ok(())
}

fn run_command(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if !status.success() { panic!("Command failed to execute: {:?}", command); }
    Ok(())
}
