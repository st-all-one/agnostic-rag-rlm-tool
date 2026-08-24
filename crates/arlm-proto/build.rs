use std::io::Result;
use std::time::Instant;

fn main() -> Result<()> {
    let proto_root = "proto";

    let proto_files = [
        format!("{proto_root}/project.proto"),
        format!("{proto_root}/index.proto"),
        format!("{proto_root}/search.proto"),
        format!("{proto_root}/context.proto"),
        format!("{proto_root}/session.proto"),
        format!("{proto_root}/server.proto"),
        format!("{proto_root}/auth.proto"),
        format!("{proto_root}/query_cache.proto"),
        format!("{proto_root}/service.proto"),
    ];

    // Generated code is external (prost/tonic output); the `proto` module in
    // lib.rs carries `#![allow(...)]` so workspace pedantic lints stay clean.
    let cfg = tonic_build::configure()
        .build_server(true)
        .build_client(true);

    let stage_start = Instant::now();
    cfg.compile_protos(&proto_files, &[proto_root])?;
    let stage_ms = stage_start.elapsed().as_millis();

    eprintln!(
        "[arlm-proto/build] stage=compile_protos duration_ms={stage_ms} files={}",
        proto_files.len()
    );

    Ok(())
}
