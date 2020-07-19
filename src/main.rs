mod code;

use std::env;
use std::str;
use std::time::Duration;

use serenity::client::Client;
use serenity::model::channel::Message;
use serenity::prelude::{Context, EventHandler};
use serenity::utils::MessageBuilder;

use lazy_static::lazy_static;
use regex::Regex;
use tokio;

use anyhow;

// TODO: My error module is a little cumbersome. Maybe just use anyhow? Not as nice for user errors
// tho. Need to read more

use crate::code::CodeRunner;

// TODO: Put this in it's own module
// TODO: Allow generic strings
#[derive(Debug)]
struct UserError(String);

use std::error::Error;
use std::fmt;

// TODO: Can I have from and into called automatically?

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", &self.0)
    }
}

impl Error for UserError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

struct Handler {
    runner: CodeRunner,
}

lazy_static! {
    // TODO: Catch if someone just forgot to specify a language? Probably just involves make lang
    // use * and report errors if it's empty
    static ref CODE_BLOCK: Regex =
        Regex::new(r"(?sm)```(?P<lang>\S+)\n(?P<code>.*)```").unwrap();
}

impl Handler {
    async fn message_impl(&self, ctx: &Context, msg: &Message) -> anyhow::Result<()> {
        log::debug!("message: {:?}", msg.content);
        let (lang, code) = match CODE_BLOCK.captures(&msg.content) {
            Some(caps) => (
                caps.name("lang").unwrap().as_str(),
                caps.name("code").unwrap().as_str(),
            ),
            None => {
                return Err(UserError(
                    r"Were you trying to run some code? I couldn't find any code blocks in your message.

Be sure to annotate your code blocks with a language like
\`\`\`python
print('Hello World')
\`\`\`".into(),
                ).into());
            }
        };
        log::debug!("language: {:?}, code: {:?}", lang, code);
        msg.react(&ctx, '🤖').await?;
        let output = self.runner.run_code(lang, code).await?;
        let mut reply = MessageBuilder::new();
        if !output.status.success() {
            reply.push_bold("EXIT STATUS: ");
            match output.status.code() {
                Some(code) => reply.push_line(code),
                None => reply.push_line("terminated by signal"),
            };
        }
        if !output.stdout.is_empty() {
            // I like to keep output simple if there's no stderr
            if !output.stderr.is_empty() {
                reply.push_bold("STDOUT:");
            }
            reply.push_codeblock(str::from_utf8(&output.stdout).unwrap(), None);
        }
        if !output.stderr.is_empty() {
            reply
                .push_bold("STDERR:")
                .push_codeblock(str::from_utf8(&output.stderr).unwrap(), None);
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
                msg.reply(&ctx, e).await.unwrap();
            }
            Ok(()) => {}
        }
    }
}

// TODO: Periodically prune things maybe? Probably not

#[tokio::main]
async fn main() {
    use simple_logger;
    simple_logger::init_with_level(log::Level::Info).unwrap();

    // Login with a bot token from the environment
    let mut client = Client::new(&env::var("DISCORD_TOKEN").unwrap())
        .event_handler(Handler {
            // TODO: Make it more clear what 15 is. It's timeout in seconds. Maybe with_timeout?
            runner: CodeRunner::with_timeout(Duration::from_secs(15)).await,
        })
        .await
        .unwrap();

    // Start as many shards as Discord recommends
    client.start_autosharded().await.unwrap();
}
