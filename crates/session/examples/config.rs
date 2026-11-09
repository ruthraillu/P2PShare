fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("Pass the config.toml path"))?;
    let _ = p2pshare_session::AppConfig::load_or_create(&path, "Windows")?;
    println!("Configuration ready: {}", path.display());
    Ok(())
}
