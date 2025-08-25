use std::io::Result;

fn main() -> Result<()> {
    // Compile protobuf files for cluster communication
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(&["proto/audio_cluster.proto"], &["proto/"])?;

    // Rerun build script if proto files change
    println!("cargo:rerun-if-changed=proto/audio_cluster.proto");
    
    Ok(())
}