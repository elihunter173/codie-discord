mod db;
mod discord;
mod lang;
mod logging;
mod options_parser;
mod runner;

use std::{collections::HashMap, env, time::Duration};

use serenity::client::Client;
use shiplift::Docker;

use crate::{db::MessageIds, discord::Handler, lang::LangRef, runner::CodeRunner};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,codie=info"))
        .init();

    let mut langs = HashMap::new();
    let mut language_text = Vec::new();
    for &lang in inventory::iter::<LangRef> {
        log::info!(
            "Registering language `{}` with codes {:?}",
            lang,
            lang.codes()
        );
        let mut codes = Vec::new();
        for &c in lang.codes() {
            if let Some(old_lang) = langs.insert(c, lang) {
                panic!("{} and {} have the same code {:?}", old_lang, lang, c);
            }
            codes.push(format!("{}", c));
        }
        language_text.push(format!("**{}:** {}", lang, codes.join(", ")));
    }
    // inventory::iter iterates in reverse order
    language_text.reverse();

    let db = sled::open("data")?;

    // Login with a bot token from the environment
    let mut client = Client::builder(&env::var("DISCORD_TOKEN").expect("`DISCORD_TOKEN` not set"))
        .event_handler(Handler {
            language_text: language_text.join("\n").into_boxed_str(),
            bot: CodeRunner {
                docker: Docker::new(),
                langs,
                timeout: Duration::from_secs(30),
                cpus: 1.0,
                memory: 128 * 1024 * 1024,
            },
            message_ids: MessageIds::new(db.open_tree("message_ids")?),
        })
        .await?;

    // Start as many shards as Discord recommends
    client.start_autosharded().await?;

    Ok(())
}
