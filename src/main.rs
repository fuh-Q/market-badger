use std::sync::Arc;

use serenity::all::{Http, MessageUpdateEvent, ShardManager, UserId};
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;

const DANK_MEMER: u64 = 270904126974590976;

struct Owner;
impl TypeMapKey for Owner {
    type Value = UserId;
}

struct ClientShards;
impl TypeMapKey for ClientShards {
    type Value = Arc<ShardManager>;
}

struct MarketCodeFlow;
impl TypeMapKey for MarketCodeFlow {
    type Value = Box<dyn Iterator<Item = &'static str> + Send + Sync>;
}

struct Handler;
#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.content != "!ping" {
            return;
        }

        let data = ctx.data.read().await;
        let shard_mngr = data
            .get::<ClientShards>()
            .expect("expected shard manager as a key");

        let runners = shard_mngr.runners.lock().await;
        let shard = match runners.get(&ctx.shard_id) {
            Some(runner) => runner,
            None => {
                if let Err(e) = msg.reply_ping(&ctx.http, "internal error").await {
                    eprintln!("could not send msg: {e}");
                }
                return;
            }
        };

        let fmt = |l: std::time::Duration| format!("`{:.2}ms`", l.as_secs_f32() * 1000f32);
        let res = match shard.latency {
            Some(latency) => msg.reply_ping(&ctx.http, fmt(latency)).await,
            None => msg.reply_ping(&ctx.http, "pong").await,
        };

        if let Err(e) = res {
            eprintln!("could not send message: {e}");
        }
    }

    async fn message_update(
        &self,
        ctx: Context,
        old: Option<Message>,
        _new: Option<Message>,
        _: MessageUpdateEvent,
    ) {
        let msg = match old {
            Some(msg) => msg,
            // if it's none, then the message cache is broken
            None => return,
        };

        // dank memer sends all messages using embeds
        // we can assume that msg.embeds[0] exists
        let t = msg.embeds[0].title.as_ref();
        if msg.author.id != DANK_MEMER
            || t.is_none()
            || t.is_some_and(|s| s != "Pending Confirmation")
        {
            return;
        }

        let invocation = match msg.referenced_message {
            Some(refmsg) => refmsg,
            None => return,
        };

        if !invocation
            .content
            .to_lowercase()
            .starts_with("pls market accept")
        {
            return;
        }

        {
            let data = ctx.data.read().await;
            let owner = data.get::<Owner>().expect("owner's user ID not found");

            if &invocation.author.id != owner {
                return;
            }
        }

        let code = {
            let mut data = ctx.data.write().await;
            let flow = data
                .get_mut::<MarketCodeFlow>()
                .expect("flow iterator not found");

            flow.next().expect("market code flow was empty...?")
        };

        let content = format!("```\npls market accept {code} 1\n```");
        let res = invocation.reply_ping(&ctx, content).await;
        if let Err(e) = res {
            eprintln!("failed to send next flow code: {e}");
        }
    }

    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} ({}) connected", ready.user.name, ready.user.id);
    }
}

#[tokio::main]
async fn main() {
    let token = include_str!("../token.txt");
    let intents = GatewayIntents::default() | GatewayIntents::MESSAGE_CONTENT;

    let codes = include_str!("../codes.txt").trim();
    if codes.is_empty() {
        panic!("no market codes were given");
    }

    let codes_iter = codes.split_whitespace().filter(|l| l.starts_with("PV"));
    println!("Cycling over {} offer codes...", codes_iter.clone().count());

    let http = Http::new(&token);
    let owner = match http.get_current_application_info().await {
        Ok(info) => {
            if let Some(team) = info.team {
                team.owner_user_id
            } else if let Some(owner) = &info.owner {
                owner.id
            } else {
                panic!("could not determine bot owner")
            }
        }
        Err(e) => panic!("could not fetch app info: {e}"),
    };

    let mut cache_config = serenity::cache::Settings::default();
    cache_config.max_messages = 100;

    let mut client = match Client::builder(&token, intents)
        .event_handler(Handler)
        .cache_settings(cache_config)
        .type_map_insert::<Owner>(owner)
        .type_map_insert::<MarketCodeFlow>(Box::new(codes_iter.cycle()))
        .await
    {
        Ok(c) => c,
        Err(e) => panic!("failed creating client: {e}"),
    };

    {
        let mut data = client.data.write().await;
        data.insert::<ClientShards>(Arc::clone(&client.shard_manager));
        data.get_mut::<MarketCodeFlow>()
            .expect("flow iterator not found")
            .next();
    }

    if let Err(e) = client.start().await {
        panic!("client could not start: {e}");
    }
}
