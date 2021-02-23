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
use tokio::sync::mpsc::{self, Sender};

use crate::{
    db::MessageIds,
    options_parser::parse_options,
    runner::{CodeRunner, UnrecognizedContainer},
};

// TODO: Do I want to react to message when I send them?

pub struct Handler {
    pub language_text: Box<str>,
    pub bot: CodeRunner,
    pub message_ids: MessageIds,
}

async fn should_run(ctx: &Context, msg: &Message) -> bool {
    msg.content.starts_with("#!run") || msg.mentions_me(ctx).await.unwrap()
}

#[derive(Debug, Eq, PartialEq)]
struct RunMessage<'a> {
    opts: &'a str,
    lang: &'a str,
    code: &'a str,
}

fn parse_message(msg: &str) -> Option<RunMessage> {
    // (?s) enables the 's' flag which lets . match '\n'
    static MENTION_CODE_LANG: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?s)```(?P<lang>\S*)\n(?P<code>.*)```").unwrap());
    static MENTION_OPTS: Lazy<Regex> = Lazy::new(|| Regex::new(r"`\[\[(?P<opts>.*)\]\]`").unwrap());

    static CMD_RUN_ALL: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?s)^#!run\s+((?P<opts>.*)\s+)?```(?P<lang>\S*)\n(?P<code>.*)```").unwrap()
    });

    if msg.starts_with("#!run") {
        CMD_RUN_ALL.captures(msg).map(|caps| RunMessage {
            opts: caps.name("opts").map(|s| s.as_str()).unwrap_or(""),
            lang: caps.name("lang").unwrap().as_str(),
            code: caps.name("code").unwrap().as_str(),
        })
    } else {
        let (lang, code) = match MENTION_CODE_LANG.captures(msg) {
            Some(caps) => (
                caps.name("lang").unwrap().as_str(),
                caps.name("code").unwrap().as_str(),
            ),
            None => return None,
        };
        let opts = match MENTION_OPTS.captures(msg) {
            Some(caps) => caps.name("opts").unwrap().as_str(),
            None => "",
        };
        Some(RunMessage { opts, lang, code })
    }
}

// XXX: Ideally this would use generators rather than a channel...
async fn try_run_raw(runner: &CodeRunner, msg: &str, tx: Sender<String>) {
    macro_rules! send {
        ($($arg:tt)*) => ( tx.send(format!($($arg)*)).await.unwrap() )
    }
    macro_rules! bail {
        ($($arg:tt)*) => ( return send!($($arg)*) )
    }

    log::debug!("Responding to {:#?}", msg);
    let run = match parse_message(msg) {
        Some(run) => run,
        None => bail!(
            r"Were you trying to run some code? I couldn't find any code blocks in your message.

Be sure to annotate your code blocks with a language like
\`\`\`python
print('Hello World')
\`\`\`"
        ),
    };
    if run.lang.is_empty() {
        bail!(
            r"I noticed you sent a code block but didn't include a language tag, so I don't know how to run it. The language goes immediately after the \`\`\` like so

\`\`\`your-language-here
{code}\`\`\`",
            code = run.code
        );
    }
    let opts = match parse_options(run.opts) {
        Ok(opts) => opts,
        // TODO: Improve error messages
        Err(err) => bail!("{}", err),
    };

    log::debug!("{:?}", run);
    let lang_ref = match runner.get_lang_by_code(run.lang) {
        Some(lang) => lang,
        // TODO: Get suggestions using strsim
        None => bail!(
            "I'm sorry. I don't know how to run `{}` code snippets.",
            run.lang,
        ),
    };

    let run_spec = match lang_ref.run_spec(opts) {
        Ok(run_spec) => run_spec,
        Err(err) => bail!("{}", err),
    };
    match runner.run_code(&run_spec, run.code).await {
        Ok(output) => send!("{}", output),
        Err(err) => match err.downcast_ref::<UnrecognizedContainer>() {
            Some(_) => {
                send!("Building container. Please be patient. This may take awhile.");
                if let Err(err) = runner.build(&run_spec).await {
                    bail!("{}", err);
                }
                match runner.run_code(&run_spec, run.code).await {
                    Ok(output) => send!("{}", output),
                    Err(err) => bail!("{}", err),
                }
            }
            None => bail!("{}", err),
        },
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
        // TODO: (This is a discord bug.) For some reason she seems to ping the user when the
        // message is updated but not when it is initially sent. Preferably she would never ping
        // the user because that can be annoying.

        let msg = event
            .channel_id
            .message(&ctx, event.id)
            .await
            .expect("failed to get handle on message");

        if msg.is_own(&ctx).await {
            return;
        }
        if !should_run(&ctx, &msg).await {
            return;
        }

        let reply_id = match self.message_ids.get(msg.id).unwrap() {
            Some(reply_id) => reply_id,
            None => {
                msg.react(&ctx, '❌')
                    .await
                    .expect("failed to react to message");
                return;
            }
        };

        let runner = &self.bot;
        let (tx, mut rx) = mpsc::channel(2);
        tokio::join!(
            async {
                try_run_raw(runner, &msg.content, tx).await;
            },
            async {
                while let Some(ref body) = rx.recv().await {
                    match msg
                        .channel_id
                        .edit_message(&ctx, reply_id, |builder| builder.content(body))
                        .await
                    {
                        Ok(_) => {}
                        Err(err) => {
                            msg.channel_id
                                .edit_message(&ctx, reply_id, |builder| builder.content(err))
                                .await
                                .expect("failed to edit message");
                        }
                    }
                }
            }
        );
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
        } else if should_run(&ctx, &msg).await {
            let runner = &self.bot;
            let (tx, mut rx) = mpsc::channel(2);
            tokio::join!(
                async {
                    try_run_raw(runner, &msg.content, tx).await;
                },
                async {
                    let body = rx.recv().await.expect("at least one message");
                    let mut reply = msg
                        .reply(&ctx, body)
                        .await
                        .expect("failed to reply to message");
                    if let Some(_) = self.message_ids.insert(msg.id, reply.id).unwrap() {
                        panic!("colliding message ids");
                    }
                    while let Some(ref body) = rx.recv().await {
                        match reply.edit(&ctx, |builder| builder.content(body)).await {
                            Ok(_) => {}
                            Err(err) => {
                                reply
                                    .edit(&ctx, |builder| builder.content(err))
                                    .await
                                    .expect("failed to edit message");
                            }
                        }
                    }
                }
            );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message_empty() {
        assert_eq!(parse_message(""), None);
    }

    #[test]
    fn test_parse_message_random() {
        assert_eq!(
            parse_message("Hey! I just wanted to check up on your progress on the project. Do you think you could have your part done by tomorrow?"),
            None,
        );
    }

    #[test]
    fn test_parse_message_mention_no_opts() {
        assert_eq!(
            parse_message("@Codie ```py\nprint('Hello, World!')\n```"),
            Some(RunMessage {
                lang: "py",
                opts: "",
                code: "print('Hello, World!')\n",
            }),
        );
    }

    #[test]
    fn test_parse_message_mention_opts() {
        assert_eq!(
            parse_message("@Codie `[[version=3.8]]` ```py\nprint('Hello, World!')\n```"),
            Some(RunMessage {
                lang: "py",
                opts: "version=3.8",
                code: "print('Hello, World!')\n",
            }),
        );
    }

    #[test]
    fn test_parse_message_run_cmd_no_opts() {
        assert_eq!(
            parse_message("#!run ```py\nprint('Hello, World!')\n```"),
            Some(RunMessage {
                lang: "py",
                opts: "",
                code: "print('Hello, World!')\n",
            }),
        );
    }

    #[test]
    fn test_parse_message_run_cmd_opts() {
        assert_eq!(
            parse_message("#!run version=3.8 ```py\nprint('Hello, World!')\n```"),
            Some(RunMessage {
                lang: "py",
                opts: "version=3.8",
                code: "print('Hello, World!')\n",
            }),
        );
    }

    #[test]
    fn test_parse_message_run_cmd_nospace() {
        assert_eq!(
            parse_message("#!run version=3.8```py\nprint('Hello, World!')\n```"),
            None,
        );
    }
}
