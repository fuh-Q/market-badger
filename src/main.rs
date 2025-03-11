use std::collections::HashSet;
use std::sync::Arc;

use serenity::all::{
    ActionRowComponent, ComponentInteraction, CreateActionRow, CreateButton, CreateEmbed,
    CreateInputText, CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage,
    CreateModal, Http, InputTextStyle, Interaction, InteractionType, MessageReference,
    MessageUpdateEvent, ModalInteraction, ShardManager, UserId,
};
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

struct AllCodes;
impl TypeMapKey for AllCodes {
    type Value = HashSet<&'static str>;
}

struct MarketCodeFlow;
impl TypeMapKey for MarketCodeFlow {
    type Value = Box<dyn Iterator<Item = &'static str> + Send + Sync>;
}

fn skipuntil_modal_response() -> CreateInteractionResponse {
    CreateInteractionResponse::Modal(
        CreateModal::new("skipuntil", "Offer ID to Skip to").components(vec![
            CreateActionRow::InputText(
                CreateInputText::new(InputTextStyle::Short, "Offer ID", "offerid")
                    .min_length(6)
                    .max_length(10)
                    .required(true),
            ),
        ]),
    )
}

async fn get_next_code(ctx: &Context) -> &str {
    let mut data = ctx.data.write().await;
    let flow = data
        .get_mut::<MarketCodeFlow>()
        .expect("flow iterator not found");

    flow.next().expect("market code flow was empty...?")
}

macro_rules! flow_message {
    ($builder:ident, $offer:expr) => {{
        let content = format!("```\npls market accept {} 1       \n```", $offer);

        $builder::new()
            .button(CreateButton::new("skip").label("Skip current"))
            .button(
                CreateButton::new("skipuntil")
                    .label("Skip until...")
                    .style(serenity::all::ButtonStyle::Secondary),
            )
            .embed(CreateEmbed::new().description(content))
    }};
}

struct Handler;
impl Handler {
    async fn modal_interaction(&self, ctx: Context, interaction: ModalInteraction) {
        // no need to check anything, all that was done in the component int handler
        let user_input = match interaction.data.components[0].components[0].clone() {
            ActionRowComponent::InputText(i) => i
                .value
                .expect("always 'Some' when receiving, as specified by the documentation")
                .to_uppercase(),
            _ => unreachable!("`InputText`s are the only components allowed in modals"),
        };

        {
            let data = ctx.data.read().await;
            let all_codes = data
                .get::<AllCodes>()
                .expect("market offers' HashSet not present");
            if !all_codes.contains(user_input.as_str()) {
                let builder = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("input did not match any offer specified in `codes.txt`"),
                );
                if let Err(e) = interaction.create_response(&ctx.http, builder).await {
                    eprintln!("could not send modal response: {e}");
                };

                return;
            }
        }

        let code = {
            let mut data = ctx.data.write().await;
            let flow = data
                .get_mut::<MarketCodeFlow>()
                .expect("flow iterator not found");

            let exp = "market code flow was empty...?";
            let mut code = flow.next().expect(exp);
            while code != user_input {
                code = flow.next().expect(exp);
            }

            code
        };

        let builder = flow_message!(CreateInteractionResponseMessage, code);
        if let Err(e) = interaction
            .create_response(&ctx.http, CreateInteractionResponse::Message(builder))
            .await
        {
            eprintln!("could not send modal response: {e}");
        }
    }

    async fn component_interaction(&self, ctx: Context, interaction: ComponentInteraction) {
        let id = interaction.data.custom_id.as_str();
        let owner = {
            let data = ctx.data.read().await;
            *data.get::<Owner>().expect("owner's user ID not found")
        };

        if id == "skipuntil" && interaction.user.id == owner {
            if let Err(e) = interaction
                .create_response(&ctx.http, skipuntil_modal_response())
                .await
            {
                eprintln!("could not send modal: {e}");
            }
        } else if id == "skip" && interaction.user.id == owner {
            let builder =
                flow_message!(CreateInteractionResponseMessage, get_next_code(&ctx).await);
            if let Err(e) = interaction
                .create_response(&ctx.http, CreateInteractionResponse::Message(builder))
                .await
            {
                eprintln!("failed [skip current] response: {e}");
            }
        } else if interaction.user.id != owner {
            if let Err(e) = interaction
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new().content("not your controls"),
                    ),
                )
                .await
            {
                eprintln!("failed [skip current] response: {e}");
            }
        }
    }
}

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

        let code = get_next_code(&ctx).await;
        let builder = flow_message!(CreateMessage, code).reference_message(
            // why doesn't serenity have builtin message -> messageref conversion
            MessageReference::new(
                serenity::all::MessageReferenceKind::Default,
                invocation.channel_id,
            )
            .message_id(invocation.id),
        );

        let res = invocation.channel_id.send_message(&ctx.http, builder).await;
        if let Err(e) = res {
            eprintln!("failed to send next flow code ({code}): {e}");
        }
    }

    async fn interaction_create(&self, ctx: Context, i: Interaction) {
        match i.kind() {
            InteractionType::Modal => self.modal_interaction(ctx, i.modal_submit().unwrap()).await,
            InteractionType::Component => {
                self.component_interaction(ctx, i.message_component().unwrap())
                    .await
            }
            _ => return,
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
    let all_codes = codes_iter.clone().collect::<HashSet<_>>();
    println!("Cycling over {} offer codes...", all_codes.len());

    let http = Http::new(token);
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

    let mut client = match Client::builder(token, intents)
        .event_handler(Handler)
        .cache_settings(cache_config)
        .type_map_insert::<Owner>(owner)
        .type_map_insert::<AllCodes>(all_codes)
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
