fn main() -> Result<(), Box<dyn std::error::Error>> {
    prost_build::Config::new()
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .type_attribute(
            ".pillar.ControllerCommand.command",
            "#[allow(clippy::large_enum_variant)]",
        )
        .compile_protos(&["proto/pillar.proto"], &["proto/"])?;
    Ok(())
}
