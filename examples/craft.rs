//! End-to-end: mine a log (retrying across candidates) then craft planks.
use std::collections::HashMap;
use rustcraft::bot::{Bot, BotEvent};
use rustcraft::protocol::ClientOptions;
use rustcraft::registry::{create_registry, BlockCollisionShapes, Registry};
fn count(bot:&Bot,name:&str)->i32{ let Some(d)=bot.registry.items_by_name.get(name) else {return 0}; bot.inventory.slots.iter().flatten().filter(|i|i.type_id==d.id).map(|i|i.count).sum() }
async fn gather_log<'a>(bot:&mut Bot<'a>)->std::io::Result<Option<String>>{
    for log in ["oak_log","birch_log","spruce_log"] {
        let mut tries=0;
        while tries<4 {
            tries+=1;
            let Some((x,y,z))=bot.find_block(log,48) else { break };
            bot.goto(x,y,z).await?;
            // dig the nearest reachable, in-sight log
            let Some((tx,ty,tz))=bot.find_block(log,5) else { continue };
            let before=count(bot,log);
            println!("  mining {log} ({tx},{ty},{tz})…");
            bot.dig(tx,ty,tz).await?;
            let _=bot.goto(tx,ty,tz).await?;  // walk onto the drop
            bot.wait_ticks(15).await?;
            if count(bot,log)>before { println!("  got {log}!"); return Ok(Some(log.to_string())); }
        }
    }
    Ok(None)
}
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let reg = create_registry("data","26.1.2").unwrap_or_else(|_| Registry::build(vec![],vec![],vec![],vec![],vec![],vec![],BlockCollisionShapes::default(),HashMap::new(),"26.1.2"));
    let opts = ClientOptions{host:"mc.bridgerb.com".into(),port:25565,username:format!("RC{}",std::process::id()),access_token:None,uuid:None};
    let mut bot = Bot::connect(opts,&reg).await?;
    let mut chunks=0; let mut done=false;
    loop { match bot.next_event().await? {
        Some(BotEvent::Spawn)=>println!("spawned at {:?}", bot.entity.position),
        Some(BotEvent::ChunkLoad(..))=>{ chunks+=1; if chunks>=12 && !done { done=true;
            match gather_log(&mut bot).await? {
                Some(log)=>{ let planks=log.replace("_log","_planks");
                    let pid=bot.registry.items_by_name.get(&planks).unwrap().id;
                    let r=bot.recipes_for(pid,None,false);
                    println!("crafting {planks} from {}x {log} ({} recipes)…", count(&bot,&log), r.len());
                    if let Some(rec)=r.first().cloned(){ match bot.craft(&rec,1,false).await {
                        Ok(())=>println!("CRAFTED -> {planks}={}, {log}={}", count(&bot,&planks), count(&bot,&log)),
                        Err(e)=>println!("craft err: {e}") } } }
                None=>println!("could not gather a log"),
            }
            break; } }
        Some(BotEvent::Kicked(r))=>{println!("kick {r}");break;}
        None=>{println!("eof");break;} _=>{} } }
    Ok(())
}
