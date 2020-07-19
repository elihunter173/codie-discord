mod code;

use std::env;
use std::time::Duration;

use serenity::client::Client;
use serenity::model::channel::Message;
use serenity::prelude::{Context, EventHandler};
use serenity::utils::MessageBuilder;

use lazy_static::lazy_static;
use regex::Regex;

use crate::code::CodeRunner;

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
    runner: CodeRunner,
}

lazy_static! {
    static ref CODE_BLOCK: Regex = Regex::new(r"(?sm)```(?P<lang>\S*)\n(?P<code>.*)```").unwrap();
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
        msg.react(&ctx, '🤖').await?;
        let output = self.runner.run_code(lang, code).await?;
        let mut reply = MessageBuilder::new();
        if !output.success() {
            reply.push_bold("EXIT STATUS: ").push_line(output.status);
        }
        if !output.stdout.is_empty() {
            // I like to keep output simple if there's no stderr
            if !output.stderr.is_empty() {
                reply.push_bold("STDOUT:");
            }
            reply.push_codeblock(&output.stdout, None);
        }
        if !output.stderr.is_empty() {
            reply
                .push_bold("STDERR:")
                .push_codeblock(&output.stderr, None);
        }
        msg.channel_id.say(&ctx, reply.build()).await?;
        Ok(())
    }
}

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        // TODO: Have a command for help on supported languages, on particular languages and, stuff
        // like that
        if msg.is_own(&ctx).await {
            return;
        }
        if !msg.mentions_me(&ctx).await.unwrap() {
            return;
        }

        match self.message_impl(&ctx, &msg).await {
            Err(e) => {
                log::error!("{:#?}", e);
                msg.reply(&ctx, e).await.unwrap();
            }
            Ok(()) => {}
        }
    }
}

// TODO: Periodically prune things maybe? Probably not

#[tokio::main]
async fn main() {
    simple_logger::init_with_level(log::Level::Info).unwrap();

    // Login with a bot token from the environment
    let mut client = Client::new(&env::var("DISCORD_TOKEN").unwrap())
        .event_handler(Handler {
            runner: CodeRunner::with_timeout(Duration::from_secs(15)).await,
        })
        .await
        .unwrap();

    // Start as many shards as Discord recommends
    client.start_autosharded().await.unwrap();
}
