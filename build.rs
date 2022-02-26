fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
    .type_attribute("Node", "#[derive(serde::Serialize,serde::Deserialize)]")
    .compile(
        &[
            "proto/node_functions.proto",
            "proto/discovery.proto"
        ],
        &["proto"]
    )?;

    Ok(())
}