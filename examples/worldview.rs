//! World introspection tool — connect, optionally walk to a coordinate, then
//! print an ASCII view of the terrain so I can SEE what the bot sees (basins,
//! walls, trees, water, lava) instead of guessing.
//!
//! Run: MC_HOST=… [VIEW_X=.. VIEW_Z=..] cargo run --example worldview
//!   - Top-down heightmap (surface elevation relative to the bot)
//!   - Two cross-sections (E-W and N-S) through the bot showing the vertical
//!     terrain profile + block types.

use rustcraft::bot::{Bot, BotEvent};
use rustcraft::protocol::ClientOptions;
use rustcraft::registry::{create_registry, BlockCollisionShapes, Registry};
use std::collections::HashMap;

fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

/// One-char code for a block by name (for cross-sections).
fn code(name: &str) -> char {
    if name == "air" || name == "cave_air" || name == "void_air" {
        ' '
    } else if name.contains("water") {
        '~'
    } else if name.contains("lava") {
        'L'
    } else if name.ends_with("_log") || name.ends_with("_wood") || name.contains("stem") {
        'T'
    } else if name.contains("leaves") {
        '*'
    } else if name.contains("grass_block") || name == "grass" {
        ','
    } else if name.contains("dirt") || name.contains("podzol") || name.contains("mud") {
        '.'
    } else if name.contains("sand") || name.contains("gravel") {
        ':'
    } else if name.contains("stone") || name.contains("deepslate") || name.contains("granite")
        || name.contains("diorite") || name.contains("andesite") || name.contains("tuff")
        || name.contains("ore") || name.contains("cobble") || name.contains("basalt")
    {
        '#'
    } else {
        'o'
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let host = env("MC_HOST", "localhost");
    let port: u16 = env("MC_PORT", "25565").parse().unwrap_or(25565);
    let username = env("MC_USERNAME", "worldview");
    let data_dir = env("STEVE_DATA", "data");
    let registry = create_registry(&data_dir, "26.1.2").unwrap_or_else(|_| {
        Registry::build(vec![], vec![], vec![], vec![], vec![], vec![], BlockCollisionShapes::default(), HashMap::new(), "26.1.2")
    });

    let options = ClientOptions { host, port, username, access_token: None, uuid: None };
    let mut bot = Bot::connect(options, &registry).await?;

    let mut chunks = 0;
    loop {
        match bot.next_event().await? {
            Some(BotEvent::Spawn) => println!("spawned at {:?}", bot.entity.position),
            Some(BotEvent::ChunkLoad(..)) => { chunks += 1; if chunks >= 16 { break; } }
            Some(BotEvent::Kicked(r)) => { println!("kicked: {r}"); return Ok(()); }
            None => { println!("disconnected"); return Ok(()); }
            _ => {}
        }
    }
    bot.wait_ticks(20).await?;

    // Optionally walk to a target coordinate first (to inspect a stuck spot).
    if let (Ok(vx), Ok(vz)) = (std::env::var("VIEW_X"), std::env::var("VIEW_Z")) {
        let (tx, tz): (i32, i32) = (vx.parse().unwrap_or(0), vz.parse().unwrap_or(0));
        println!("walking toward ({tx},?,{tz}) …");
        let _ = bot.goto_xz(tx, tz, 3.0).await;
        for _ in 0..40 { bot.drive_tick().await.ok(); }
    }

    let p = bot.entity.position;
    let (bx, by, bz) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    println!("\nbot at ({bx},{by},{bz})  on_ground={}\n", bot.entity.on_ground);

    // ── Top-down surface heightmap (relative elevation) ──
    let r = 24i32;
    let surface_y = |x: i32, z: i32| -> Option<i32> {
        for y in (by - 20..=by + 25).rev() {
            if bot.block_state_at(x, y, z) != 0 {
                let n = bot.block_at(x, y, z).map(|b| b.name.clone()).unwrap_or_default();
                if !n.contains("leaves") {
                    return Some(y);
                }
            }
        }
        None
    };
    let log_above = |x: i32, z: i32| -> bool {
        (by - 4..=by + 12).any(|y| bot.block_at(x, y, z).map(|b| b.name.ends_with("_log")).unwrap_or(false))
    };
    println!("TOP-DOWN heightmap (relative to bot y={by}; columns = x, rows = z; 'B'=bot)");
    println!("  digits/+ = higher, - = lower, '=' level, T=tree ~=water L=lava '·'=void");
    print!("    ");
    for dx in -r..=r { print!("{}", if dx == 0 { '|' } else { ' ' }); }
    println!();
    for dz in -r..=r {
        print!("{:4} ", bz + dz);
        for dx in -r..=r {
            let (x, z) = (bx + dx, bz + dz);
            let ch = if dx == 0 && dz == 0 {
                'B'
            } else if log_above(x, z) {
                'T'
            } else {
                match surface_y(x, z) {
                    None => '·',
                    Some(sy) => {
                        let n = bot.block_at(x, sy, z).map(|b| b.name.clone()).unwrap_or_default();
                        if n.contains("water") { '~' }
                        else if n.contains("lava") { 'L' }
                        else {
                            let d = sy - by;
                            match d {
                                d if d <= -4 => 'v',
                                -3..=-1 => '-',
                                0 => '=',
                                1..=3 => '+',
                                4..=9 => char::from_digit((d as u32).min(9), 10).unwrap_or('^'),
                                _ => '^',
                            }
                        }
                    }
                }
            };
            print!("{ch}");
        }
        println!();
    }

    // ── Cross-sections through the bot ──
    let cross = |fixed_z: bool| {
        println!("\nCROSS-SECTION {} through bot (rows=y top→down, cols={}):",
            if fixed_z { "E-W (varying x)" } else { "N-S (varying z)" },
            if fixed_z { "x" } else { "z" });
        for y in (by - 6..=by + 12).rev() {
            print!("{:4} ", y);
            for d in -r..=r {
                let (x, z) = if fixed_z { (bx + d, bz) } else { (bx, bz + d) };
                let ch = if d == 0 && y == by { 'B' } else if d == 0 && y == by + 1 { 'b' } else {
                    bot.block_at(x, y, z).map(|b| code(&b.name)).unwrap_or(' ')
                };
                print!("{ch}");
            }
            println!();
        }
    };
    cross(true);
    cross(false);
    println!("\n(legend: #=stone .=dirt ,=grass :=sand T=log *=leaves ~=water L=lava ' '=air B/b=bot)");
    Ok(())
}
