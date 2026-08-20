use std::io::Result;

fn main() -> Result<()> {
    let proto_root = "proto";

    tonic_build::configure().compile_protos(&[format!("{proto_root}/arlm.proto")], &[proto_root])?;

    Ok(())
}
