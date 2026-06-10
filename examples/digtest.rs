//! Minimal dig isolation test: connect, wait for chunks, then repeatedly try to
//! break a definitely-reachable floor block beside the bot, logging the result.
//! Run: MC_HOST=… cargo run --example digtest

use rustcraft::bot::{Bot, BotEvent};
use rustcraft::protocol::ClientOptions;
use rustcraft::registry::{create_registry, BlockCollisionShapes, Registry};
use std::collections::HashMap;

fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let host = env("MC_HOST", "localhost");
    let port: u16 = env("MC_PORT", "25565").parse().unwrap_or(25565);
    let username = env("MC_USERNAME", "digtest");
    let data_dir = env("STEVE_DATA", "data");
    let registry = create_registry(&data_dir, "26.1.2").unwrap_or_else(|_| {
        Registry::build(vec![], vec![], vec![], vec![], vec![], vec![], BlockCollisionShapes::default(), HashMap::new(), "26.1.2")
    });

    let options = ClientOptions { host, port, username, access_token: None, uuid: None };
    let mut bot = Bot::connect(options, &registry).await?;

    let mut chunks = 0;
    loop {
        match bot.next_event().await? {
            Some(BotEvent::Spawn) => println!("spawned at {:?} gamemode={}", bot.entity.position, bot.game.game_mode),
            Some(BotEvent::ChunkLoad(..)) => { chunks += 1; if chunks >= 8 { break; } }
            Some(BotEvent::Kicked(r)) => { println!("kicked: {r}"); return Ok(()); }
            None => { println!("disconnected"); return Ok(()); }
            _ => {}
        }
    }
    bot.wait_ticks(20).await?;

    let p = bot.entity.position;
    let (fx, fy, fz) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    println!("bot floor=({fx},{fy},{fz})");

    let soft = |b: &str| {
        b.contains("dirt") || b.contains("grass") || b.contains("sand") || b.contains("gravel")
            || b.ends_with("_log") || b.contains("leaves") || b.contains("snow")
    };

    // Find the nearest SOFT block (fast to break by hand) within a wide radius.
    let mut best: Option<(i32, i32, i32, String, f64)> = None;
    for dx in -16..=16 {
        for dz in -16..=16 {
            for dy in -6..=6 {
                let (x, y, z) = (fx + dx, fy + dy, fz + dz);
                if bot.block_state_at(x, y, z) != 0 {
                    if let Some(b) = bot.block_at(x, y, z) {
                        if soft(&b.name) {
                            let d = ((dx * dx + dy * dy + dz * dz) as f64).sqrt();
                            if best.as_ref().map(|bb| d < bb.4).unwrap_or(true) {
                                best = Some((x, y, z, b.name.clone(), d));
                            }
                        }
                    }
                }
            }
        }
    }
    let Some((x, y, z, name, d)) = best else { println!("no soft block found nearby"); return Ok(()); };
    println!("nearest soft block: {name} at ({x},{y},{z}) dist={d:.1} — walking adjacent");
    let _ = bot.goto_near(x, y, z, 2.0).await;
    let bp = bot.entity.position;
    println!("after walk bot=({:.1},{:.1},{:.1}) canSee={}", bp.x, bp.y, bp.z, bot.can_see_block(x, y, z));

    for attempt in 0..4 {
        if bot.block_state_at(x, y, z) == 0 {
            println!("  BROKE before attempt {attempt} ✓");
            break;
        }
        let _ = bot.dig(x, y, z).await;
        let broke = bot.block_state_at(x, y, z) == 0;
        println!("  attempt {attempt}: broke={broke} (state {})", bot.block_state_at(x, y, z));
        if broke {
            println!("  ✓✓ DIG WORKS");
            break;
        }
        bot.wait_ticks(10).await?;
    }
    println!("done");
    Ok(())
}
