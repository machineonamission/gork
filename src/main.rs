use gemini_rs;
use gemini_rs::types;
use serenity::all::{MessageReferenceKind, User};
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::prelude::*;

struct Handler;

fn user_to_string(user: &User) -> String {
    format!(
        "@{}",
        // user server nickname if it exists, otherwise global display name, otherwise username
        user.member
            .as_ref()
            .and_then(|m| m.nick.clone())
            .unwrap_or(user.display_name().to_string())
    )
}

fn format_message_contents(msg: &Message) -> String {
    let mut content = msg.content.clone();
    // replace user mentions with their usernames
    for mention in &msg.mentions {
        content = content.replace(&mention.to_string(), &user_to_string(&mention))
    }
    content
}
fn message_to_string(msg: &Message) -> String {
    let f = format!(
        "{} says:\n{}",
        user_to_string(&msg.author),
        format_message_contents(&msg)
    );
    println!("{f}");
    f
}

async fn get_reply(msg: &Message, ctx: &Context) -> Option<Message> {
    // 1) Fast path: already have an in-memory referenced message
    if let Some(rm) = msg.referenced_message.as_deref() {
        return Some(rm.clone());
    }

    // 2) Synchronously peel away to get the MessageId (or bail early)
    let id = msg
        .message_reference
        .as_ref()
        .filter(|mr| mr.kind == MessageReferenceKind::Default)
        .and_then(|mr| mr.message_id)?;

    // 3) Try the cache
    if let Some(cached) = ctx.cache.message(&msg.channel_id, id) {
        return Some(cached.clone());
    }

    // 4) Fallback to HTTP (this is the only await)
    let fetched = ctx.http.get_message(msg.channel_id, id).await.ok()?;
    Some(fetched.clone())
}

async fn trace_replies(msg: &Message, ctx: &Context) -> Vec<Message> {
    let mut out = vec![];
    let mut last = Some(msg.clone());
    while let Some(lastv) = last {
        last = get_reply(&lastv, &ctx).await;
        out.push(lastv);
    }
    out
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        let bot_self = ctx.cache.current_user().clone();
        if msg.mentions.contains(&bot_self) {
            let typing = msg.channel_id.start_typing(&ctx.http);

            let client = gemini_rs::Client::new(include_str!("../geminikey.txt"));
            let mut chat = client
                .chat("gemini-2.0-flash")
                .system_instruction(include_str!("prompt.txt"));

            for msg in trace_replies(&msg, &ctx).await.iter().rev() {
                chat.history_mut().push(
                    // push "@user says:" for non gork messages, and just format for gork messages
                    if msg.author == *bot_self {
                        types::Content {
                            role: types::Role::Model,
                            parts: vec![types::Part::text(&format_message_contents(&msg))],
                        }
                    } else {
                        types::Content {
                            role: types::Role::User,
                            parts: vec![types::Part::text(&message_to_string(&msg))],
                        }
                    },
                );
            }

            let config = chat.config_mut();
            config.temperature = Some(2.0);
            config.candidate_count = Some(1);

            let response = chat.generate_content().await.unwrap();
            let text = response.candidates[0].content.parts[0]
                .clone()
                .text
                .unwrap();

            msg.reply_ping(&ctx.http, &text).await.unwrap();
            typing.stop();
            // if msg.mentions
        }
    }
}
#[tokio::main]
async fn main() {
    // Login with a bot token from the environment
    let token = include_str!("../discordkey.txt");
    // Set gateway intents, which decides what events the bot will be notified about
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    // Create a new instance of the Client, logging in as a bot.
    let mut client = Client::builder(&token, intents)
        .event_handler(Handler)
        .await
        .expect("Err creating client");

    // Start listening for events by starting a single shard
    if let Err(why) = client.start().await {
        println!("Client error: {why:?}");
    }
}
