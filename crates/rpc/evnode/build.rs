fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protos = ["proto/evnode/v1/execution.proto"];

    tonic_build::configure()
        .build_server(true)
        .build_client(false) // We only need the server
        .out_dir("src/generated")
        .compile_protos(&protos, &["proto"])?;

    // Rerun if proto files change
    for proto in &protos {
        println!("cargo:rerun-if-changed={}", proto);
    }

    Ok(())
}
