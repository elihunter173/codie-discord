mod bot;
mod lang;

use std::convert::TryInto;
use std::env;
use std::time::Duration;
use std::{borrow::Cow, collections::HashMap};

use once_cell::sync::Lazy;
use serenity::client::Client;
use serenity::model::{channel::Message, event::MessageUpdateEvent, guild::Guild, id::MessageId};
use serenity::prelude::{Context, EventHandler};
use serenity::utils::MessageBuilder;

use shiplift::Docker;

use sled::Tree;

use regex::Regex;

use crate::bot::{CodeRunner, Output};
use crate::lang::LangRef;

struct Handler {
    bot: CodeRunner,
    message_ids: MessageIds,
}

static CODE_BLOCK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?sm)```(?P<lang>\S*)\n(?P<code>.*)```").unwrap());

fn output_message(output: &Output) -> String {
    // I use this rather tha push_codeblock_safe because it just strips out backticks but this makes it
    // look similar
    // Replace backticks with something that look really similar
    fn escape_codeblock(code: &str) -> Cow<str> {
        static CODE_BLOCK_FENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"```").unwrap());
        CODE_BLOCK_FENCE.replace_all(code, "ˋˋˋ")
    }

    let mut message = MessageBuilder::new();
    if !output.success() {
        message.push_bold("EXIT STATUS: ").push_line(output.status);
    }
    if !output.stdout.is_empty() {
        // I like to keep output simple if there's no stderr
        if !output.stderr.is_empty() {
            message.push_bold("STDOUT:");
        }
        message.push_codeblock(escape_codeblock(&output.stdout), None);
    }
    if !output.stderr.is_empty() {
        message
            .push_bold("STDERR:")
            .push_codeblock(escape_codeblock(&output.stderr), None);
    }
    message.build()
}

impl Handler {
    async fn try_run_raw(&self, msg: &str) -> String {
        log::debug!("Responding to {:#?}", msg);
        let (lang, code) = match CODE_BLOCK.captures(msg) {
            Some(caps) => (
                caps.name("lang").unwrap().as_str(),
                caps.name("code").unwrap().as_str(),
            ),
            None => {
                let message = r"Were you trying to run some code? I couldn't find any code blocks in your message.

Be sure to annotate your code blocks with a language like
\`\`\`python
print('Hello World')
\`\`\`";
                return message.into();
            }
        };
        if lang.is_empty() {
            return format!(
                r"I noticed you sent a code block but didn't include a language tag, so I don't know how to run it. The language goes immediately after the \`\`\` like so

\`\`\`your-language-here
{code}\`\`\`",
                code = code
            );
        }

        log::debug!("language: {:?}, code: {:?}", lang, code);
        let lang = match self.bot.get_lang_by_code(lang) {
            Some(lang) => lang,
            None => {
                return format!(
                    "I'm sorry, I don't know how to run `{}` code-snippets",
                    lang
                );
            }
        };

        let output = match self.bot.run_code(lang, code).await {
            Ok(output) => output,
            Err(err) => {
                return format!("{}", err);
            }
        };
        output_message(&output)
    }
}

#[serenity::async_trait]
impl EventHandler for Handler {
    // RULES FOR EDITING:
    // * The rules for mentioning is the same
    // * If a message is edited, she updates her reply to that message if it exists. Otherwise she
    // complains. Should be able to do channel.messages(|builder| builder.after(msg.id).limit(10)).
    // Might have to embed a message ID we're replying to in the thing. Or actually pick the first
    // message within 10 after that mentions the user
    // I'm kinda leaning towards message ID
    async fn message_update(
        &self,
        ctx: Context,
        _old_if_available: Option<Message>,
        _new: Option<Message>,
        event: MessageUpdateEvent,
    ) {
        // TODO: Remove all reactions upon update?
        // TODO: If it fails to find an previous message reply with a "X"

        // Says message is private or metions us
        let my_id = ctx.cache.current_user_id().await;
        let mentions_me = if let Some(mentions) = &event.mentions {
            mentions.iter().any(|user| user.id == my_id)
        } else {
            false
        };

        // Don't respond if it's a group message that doesn't mention us
        if event.guild_id.is_some() && !mentions_me {
            return;
        }

        let reply_id = self.message_ids.get(event.id).unwrap().expect("no reply");
        use serenity::model::misc::Mentionable;
        let mention = event.author.unwrap().mention();
        event
            .channel_id
            .edit_message(&ctx, reply_id, |builder| {
                builder.content(format!("{}: Re-running code", mention))
            })
            .await
            .unwrap();
        let body = self.try_run_raw(&event.content.as_ref().unwrap()).await;
        event
            .channel_id
            .edit_message(&ctx, reply_id, |builder| {
                builder.content(format!("{}: {}", mention, body))
            })
            .await
            .unwrap();
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.is_own(&ctx).await {
            return;
        }

        log::debug!("{}", msg.content);

        // TODO: Extract commands to be separate from the handler?
        // TODO: Get suggestions when you make a typo on a command using strsim
        match msg.content.split(' ').collect::<Vec<_>>().as_slice() {
            ["#!help"] => {
                msg.channel_id.say(&ctx, self.bot.help()).await.unwrap();
            }

            ["#!help", lang] => match self.bot.get_lang_by_code(lang) {
                Some(lang) => {
                    msg.channel_id.say(&ctx, lang.help()).await.unwrap();
                }
                None => {
                    msg.reply(&ctx, format!("I'm sorry. I don't know `{}`.", lang))
                        .await
                        .unwrap();
                }
            },

            ["#!languages"] => {
                msg.channel_id
                    .say(&ctx, self.bot.help_languages())
                    .await
                    .unwrap();
            }

            _ => {
                if msg.is_private() || msg.mentions_me(&ctx).await.unwrap() {
                    msg.react(&ctx, '🤖').await.unwrap();
                    let body = self.try_run_raw(&msg.content).await;
                    let reply = msg.reply(&ctx, body).await.unwrap();
                    if let Some(_) = self.message_ids.insert(msg.id, reply.id).unwrap() {
                        panic!("colliding message ids");
                    }
                }

                if msg.content.starts_with("#!") {
                    msg.reply(&ctx, "I'm sorry. I didn't recognize that command")
                        .await
                        .unwrap();
                }
            }
        }
    }

    async fn guild_create(&self, ctx: Context, guild: Guild, is_new: bool) {
        if !is_new {
            return;
        }

        // Pick the channel to send the intro message into. We want to pick the most popular
        // channel (e.g. #general) we can send messages in. Since Discord doesn't provide a native
        // way to do this, we use the heuristic of picking the channel nearest the top. I would do
        // this with stream min/max, but I couldn't get streams/iterators to work, so I had to
        // resort to this code :(
        let channel = {
            let me = ctx.cache.current_user_id().await;
            use serenity::model::channel::ChannelType;
            let text_channels = guild
                .channels
                .values()
                .filter(|chan| chan.kind == ChannelType::Text);

            let mut cur_top = None;
            for chan in text_channels {
                let can_send_messages = chan
                    .permissions_for_user(&ctx, me)
                    .await
                    .unwrap()
                    .send_messages();
                if !can_send_messages {
                    continue;
                }

                cur_top = match cur_top {
                    None => Some(chan),
                    Some(cur_top) if chan.position < cur_top.position => Some(chan),
                    _ => cur_top,
                }
            }
            cur_top
        };

        // TODO: Maybe put a delay so that we don't beat Discord intro message for us?
        if let Some(channel) = channel {
            log::info!("Saying hi to {:?}", guild.id);
            channel.say(&ctx, self.bot.help()).await.unwrap();
        }
    }
}

struct MessageIds(Tree);

// TODO: Figure out a better way to do this?
impl MessageIds {
    fn insert(&self, k: MessageId, v: MessageId) -> sled::Result<Option<MessageId>> {
        self.0
            .insert(&k.as_u64().to_le_bytes(), &v.as_u64().to_le_bytes())
            .map(|opt| {
                opt.map(|ivec| MessageId(u64::from_le_bytes(ivec.as_ref().try_into().unwrap())))
            })
    }

    fn get(&self, k: MessageId) -> sled::Result<Option<MessageId>> {
        self.0.get(&k.as_u64().to_le_bytes()).map(|opt| {
            opt.map(|ivec| MessageId(u64::from_le_bytes(ivec.as_ref().try_into().unwrap())))
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,codie=info"))
        .init();

    let mut langs = HashMap::new();
    for &lang in inventory::iter::<LangRef> {
        log::info!(
            "Registering language `{}` with codes {:?}",
            lang,
            lang.codes()
        );
        for &c in lang.codes() {
            if let Some(old_lang) = langs.insert(c, lang) {
                panic!("{} and {} have the same code {:?}", old_lang, lang, c);
            }
        }
    }

    let db = sled::open("data")?;

    // Login with a bot token from the environment
    let mut client = Client::builder(&env::var("DISCORD_TOKEN").expect("`DISCORD_TOKEN` not set"))
        .event_handler(Handler {
            bot: CodeRunner {
                docker: Docker::new(),
                langs,
                timeout: Duration::from_secs(30),
                cpus: 1.0,
                memory: 128 * 1024 * 1024,
            },
            message_ids: MessageIds(db.open_tree("message_ids")?),
        })
        .await?;

    // Start as many shards as Discord recommends
    client.start_autosharded().await?;

    Ok(())
}
