mod bot;

use std::borrow::Cow;
use std::env;
use std::time::Duration;

use serenity::client::Client;
use serenity::model::channel::Message;
use serenity::model::guild::Guild;
use serenity::prelude::*;
use serenity::utils::MessageBuilder;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::bot::{Bot, Language, Output};

// TODO: Put this in it's own module
#[derive(Debug)]
struct UserError<S: AsRef<str> + fmt::Debug>(S);

use std::error::Error;
use std::fmt;

impl<S: AsRef<str> + fmt::Debug> fmt::Display for UserError<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_ref())
    }
}

impl<S: AsRef<str> + fmt::Debug> Error for UserError<S> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

struct Handler {
    bot: Bot,
}

static CODE_BLOCK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?sm)```(?P<lang>\S*)\n(?P<code>.*)```").unwrap());

// I use this rather tha push_codeblock_safe because it just strips out backticks but this makes it
// look similar
// Replace backticks with something that look really similar
fn escape_codeblock(code: &str) -> Cow<str> {
    static CODE_BLOCK_FENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"```").unwrap());
    CODE_BLOCK_FENCE.replace_all(code, "ˋˋˋ")
}

fn output_message(output: &Output) -> String {
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
    async fn message_impl(&self, ctx: &Context, msg: &Message) -> anyhow::Result<()> {
        log::debug!("Responding to {:#?}", msg.content);
        let (lang, code) = match CODE_BLOCK.captures(&msg.content) {
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
                return Err(UserError(message).into());
            }
        };
        if lang.is_empty() {
            return Err(UserError(
                    format!(r"I noticed you sent a code block but didn't include a language tag, so I don't know how to run it. The language goes immediately after the \`\`\` like so

\`\`\`your-language-here
{code}\`\`\`", code=code),
                ).into());
        }

        log::debug!("language: {:?}, code: {:?}", lang, code);
        let lang = match Language::from_code(lang) {
            Some(lang) => lang,
            None => {
                return Err(
                    UserError(format!("I'm sorry, I don't know how to run {}", lang)).into(),
                )
            }
        };

        msg.react(&ctx, '🤖').await?;
        let output = self.bot.run_code(lang, code).await?;
        msg.channel_id.say(&ctx, output_message(&output)).await?;
        Ok(())
    }
}

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.is_own(&ctx).await {
            return;
        }

        log::debug!("{}", msg.content);

        // TODO: Extract commands to be separate from the handler
        // TODO: Handle extract this out. Use some sort of macro to define commands in a framework
        // style
        // TODO: Get suggestsions when you make a typo on a command using strsim
        match msg.content.split(' ').collect::<Vec<_>>().as_slice() {
            ["#!help"] => {
                msg.channel_id.say(&ctx, self.bot.help()).await.unwrap();
            }
            ["#!languages"] => {
                msg.channel_id
                    .say(&ctx, self.bot.help_languages())
                    .await
                    .unwrap();
            }
            ["#!help", lang] => match Language::from_code(lang) {
                Some(lang) => {
                    msg.channel_id
                        .say(&ctx, self.bot.help_lang(lang))
                        .await
                        .unwrap();
                }
                None => {
                    msg.reply(&ctx, format!("I'm sorry. I don't know `{}`.", lang))
                        .await
                        .unwrap();
                }
            },
            _ => {
                if msg.is_private() || msg.mentions_me(&ctx).await.unwrap() {
                    if let Err(e) = self.message_impl(&ctx, &msg).await {
                        log::error!("{:#?}", e);
                        msg.reply(&ctx, e).await.unwrap();
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

        // Lower position number is top Find a channel I can send messages in. I couldn't get
        // streams/iterators to work so I had to resort to this code. I know it's messy :(
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

        // TODO: Maybe put a delay?
        if let Some(channel) = channel {
            log::info!("Saying hi to {:?}", guild.id);
            channel.say(&ctx, self.bot.help()).await.unwrap();
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,codie=info"))
        .init();

    // Login with a bot token from the environment
    let mut client = Client::builder(&env::var("DISCORD_TOKEN").expect("`DISCORD_TOKEN` not set"))
        .event_handler(Handler {
            bot: Bot::new(Duration::from_secs(30), 1.0, 128 * 1024 * 1024).await,
        })
        .await?;

    // Start as many shards as Discord recommends
    client.start_autosharded().await?;

    Ok(())
}
