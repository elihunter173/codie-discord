mod discord;
mod lang;
mod options_parser;
mod runner;

use std::{collections::HashMap, env, time::Duration};

use serde::Deserialize;
use serenity::client::Client;
use shiplift::Docker;

use crate::{
    discord::{Handler, MessageIds},
    lang::LangRef,
    runner::DockerRunner,
};

#[derive(Deserialize)]
struct Config {
    log_filter: String,
    docker: DockerConfig,
    discord_token: String,
}

#[derive(Deserialize)]
struct DockerConfig {
    timeout_secs: u64,
    memory_bytes: u64,
    cpus: f64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let conf_path = env::args_os().nth(1).unwrap();
    let conf_text = std::fs::read_to_string(conf_path)?;
    let conf: Config = toml::from_str(&conf_text).unwrap();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(conf.log_filter))
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
    language_text.sort();

    let db = sled::open("data")?;

    // Login with a bot token from the environment
    let mut client = Client::builder(&conf.discord_token)
        .event_handler(Handler {
            language_text: language_text.join("\n").into_boxed_str(),
            runner: DockerRunner {
                docker: Docker::new(),
                langs,
                timeout: Duration::from_secs(conf.docker.timeout_secs),
                cpus: conf.docker.cpus,
                memory_bytes: conf.docker.memory_bytes,
            },
            message_ids: MessageIds::new(db.open_tree("message_ids")?),
        })
        .await?;

    // Start as many shards as Discord recommends
    client.start_autosharded().await?;

    Ok(())
}
