//! Mine the nearest reachable log, then craft planks. Single close-range attempt.
use std::collections::HashMap;
use rustcraft::bot::{Bot, BotEvent};
use rustcraft::protocol::ClientOptions;
use rustcraft::registry::{create_registry, BlockCollisionShapes, Registry};
fn count(bot:&Bot,name:&str)->i32{ let Some(d)=bot.registry.items_by_name.get(name) else {return 0}; bot.inventory.slots.iter().flatten().filter(|i|i.type_id==d.id).map(|i|i.count).sum() }
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let reg = create_registry("data","26.1.2").unwrap_or_else(|_| Registry::build(vec![],vec![],vec![],vec![],vec![],vec![],BlockCollisionShapes::default(),HashMap::new(),"26.1.2"));
    let opts = ClientOptions{host:"mc.bridgerb.com".into(),port:25565,username:format!("RC{}",std::process::id()),access_token:None,uuid:None};
    let mut bot = Bot::connect(opts,&reg).await?;
    let mut chunks=0; let mut done=false;
    loop { match bot.next_event().await? {
        Some(BotEvent::Spawn)=>println!("spawned {:?}", bot.entity.position),
        Some(BotEvent::ChunkLoad(..)) if !done => { chunks+=1; if chunks>=14 { done=true;
            // nearest reachable log among types, within 24
            let mut best=None; let mut bd=1e9;
            let p=bot.entity.position;
            for log in ["oak_log","birch_log","spruce_log"] {
                for (x,y,z) in bot.find_blocks(log,24,8) {
                    let d=((x as f64-p.x).powi(2)+(z as f64-p.z).powi(2)).sqrt()+3.0*((y as f64-p.y).abs());
                    if d<bd { bd=d; best=Some((log.to_string(),x,y,z)); } } }
            let Some((log,_,_,_))=best.clone() else { println!("no log near"); break; };
            for attempt in 0..3 {
                let Some((x,y,z))=bot.find_block(&log,24) else { break };
                bot.goto(x,y,z).await?;
                let Some((tx,ty,tz))=bot.find_block(&log,5) else { println!("attempt {attempt}: not in reach after goto"); continue };
                let before=count(&bot,&log);
                println!("attempt {attempt}: mining {log} ({tx},{ty},{tz}) from {:?}", bot.entity.position);
                bot.dig(tx,ty,tz).await?;
                let _=bot.goto(tx,ty,tz).await?; bot.wait_ticks(15).await?;
                if count(&bot,&log)>before {
                    let planks=log.replace("_log","_planks");
                    let pid=bot.registry.items_by_name.get(&planks).unwrap().id;
                    let r=bot.recipes_for(pid,None,false);
                    println!("got {log}={}; crafting {planks} ({} recipes)…", count(&bot,&log), r.len());
                    if let Some(rec)=r.first().cloned(){ match bot.craft(&rec,1,false).await {
                        Ok(())=>println!("CRAFTED -> {planks}={}, {log}={}", count(&bot,&planks), count(&bot,&log)),
                        Err(e)=>println!("craft err: {e}") } }
                    break;
                }
            }
            break; } }
        Some(BotEvent::Kicked(r))=>{println!("kick {r}");break;}
        None=>{println!("eof");break;} _=>{} } }
    Ok(())
}
