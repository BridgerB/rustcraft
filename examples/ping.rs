use rustcraft::protocol::ping;
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let host = args.next().unwrap_or_else(|| "localhost".into());
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(25565);
    let r = ping(&host, port).await?;
    println!("version: {} (protocol {})", r.version_name, r.protocol);
    println!("players: {}/{}", r.players_online, r.players_max);
    println!("latency: {}ms", r.latency_ms);
    Ok(())
}
