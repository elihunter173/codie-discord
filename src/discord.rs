use once_cell::sync::Lazy;
use regex::Regex;
use serenity::{
    model::{
        channel::Message,
        event::MessageUpdateEvent,
        gateway::{Activity, Ready},
    },
    prelude::{Context, EventHandler},
    utils::Color,
};

use crate::{db::MessageIds, runner::CodeRunner};

pub struct Handler {
    pub language_text: Box<str>,
    pub bot: CodeRunner,
    pub message_ids: MessageIds,
}

// TODO: When there is no code given, Discord says that the language code is the code while we say
// the language code is the language code. We should agree with discord always
static CODE_BLOCK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?sm)```(?P<lang>\S*)\n(?P<code>.*)```").unwrap());

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
                // TODO: Get suggestions using strsim
                return format!(
                    "I'm sorry. I don't know how to run `{}` code snippets.",
                    lang
                );
            }
        };

        match self.bot.run_code(lang, code).await {
            Ok(output) => format!("{}", output),
            Err(err) => format!("{}", err),
        }
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

        let reply_id = if let Some(reply_id) = self.message_ids.get(event.id).unwrap() {
            reply_id
        } else {
            let msg = event
                .channel_id
                .message(&ctx, event.id)
                .await
                .expect("failed to get handle on message");
            msg.react(&ctx, '❌')
                .await
                .expect("failed to reach to message");
            return;
        };

        event
            .channel_id
            .edit_message(&ctx, reply_id, |builder| builder.content("Re-running code"))
            .await
            .expect("failed to edit message");
        let body = self
            .try_run_raw(&event.content.as_ref().expect("failed to find message body"))
            .await;

        match event
            .channel_id
            .edit_message(&ctx, reply_id, |builder| builder.content(body))
            .await
        {
            Ok(_) => {}
            Err(err) => {
                event
                    .channel_id
                    .edit_message(&ctx, reply_id, |builder| builder.content(err))
                    .await
                    .expect("failed to edit message");
            }
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.is_own(&ctx).await {
            return;
        }

        if msg.content == "#!help" {
            // We extract this because otherwise rustfmt falis
            const HELP: &str = r#"I know how to run a variety of languages. All you have to do to ask me to run a block of code is to @ me in the message containing the code you want me to run.

Make sure to include a language right after backticks (\`\`\`) or else I won't know how to run your code!"#;
            const EXAMPLE: &str = r#"@Codie Please run this code \`\`\`python
print("Hello, World!")
\`\`\`"#;
            msg.channel_id
                .send_message(&ctx, |m| {
                    m.embed(|e| {
                        // TODO: Should I use .author instead of .title? It's smaller but it can
                        // include an icon and isn't blue
                        e.title("Codie the Code Runner")
                            .url("https://github.com/elihunter173/codie-discord")
                            .footer(|f| {
                                f.text("Made by elihunter173 with love - https://elihunter173.com/")
                            })
                            .color(Color::from_rgb(255, 105, 180))
                            .description(HELP)
                            .field("Example", EXAMPLE, true)
                            .field("Supported Languages", &self.language_text, false)
                    })
                })
                .await
                .expect("failed to send help message");
        } else if msg.is_private() || msg.mentions_me(&ctx).await.unwrap() {
            msg.react(&ctx, '🤖')
                .await
                .expect("failed to react to message");
            let body = self.try_run_raw(&msg.content).await;
            let reply = msg
                .reply(&ctx, body)
                .await
                .expect("failed to reply to message");
            if let Some(_) = self.message_ids.insert(msg.id, reply.id).unwrap() {
                panic!("colliding message ids");
            }
        } else if msg.content.starts_with("#!") {
            msg.reply(&ctx, "I'm sorry. I didn't recognize that command")
                .await
                .expect("failed to reply");
        }
    }

    async fn ready(&self, ctx: Context, _data_about_bot: Ready) {
        ctx.set_activity(Activity::listening("#!help")).await;
    }
}
