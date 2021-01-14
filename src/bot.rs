use std::str;
use std::time::Duration;
use std::{collections::HashMap, path::Path};

use futures::StreamExt;

use shiplift::tty::TtyChunk;
use shiplift::Docker;
use unicase::Ascii;

use crate::lang::LangRef;

#[derive(Debug, Eq, PartialEq)]
pub struct Output {
    pub status: u64,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status == 0
    }
}

pub struct CodeRunner {
    pub docker: Docker,
    pub langs: HashMap<Ascii<&'static str>, LangRef>,
    pub timeout: Duration,
    pub cpus: f64,
    pub memory: u64,
}

// TODO: Support options [[like this]]
// @Codie [[version="python2"]] ```py
// print "hi"
// ```
impl CodeRunner {
    pub fn get_lang_by_code(&self, code: &str) -> Option<LangRef> {
        self.langs.get(&Ascii::new(code)).copied()
    }

    pub async fn run_code<'s>(&'s self, lang: LangRef, code: &'s str) -> anyhow::Result<Output> {
        let container = {
            let response = self
                .docker
                .containers()
                .create(
                    &lang
                        .container_options()
                        .attach_stdout(true)
                        .attach_stderr(true)
                        // Run as user "nobody"
                        .user("65534:65534")
                        // Ensure that we run unprivileged
                        .capabilities(vec![])
                        .privileged(false)
                        // No internet access
                        .network_mode("none")
                        // Make it so you can just open files
                        .working_dir("/tmp")
                        .cpus(self.cpus)
                        .memory(self.memory)
                        .build(),
                )
                .await?;
            shiplift::Container::new(&self.docker, response.id)
        };
        container
            .copy_file_into(Path::new("/tmp/code"), code.as_bytes())
            .await?;

        log::info!("Starting container {}", container.id());
        container.start().await?;
        let exit = match tokio::time::timeout(self.timeout, container.wait()).await {
            Ok(result) => result?,
            Err(e) => {
                log::warn!(
                    "Force removing container {}. Reason: exceeded timeout",
                    container.id()
                );
                container
                    .remove(shiplift::RmContainerOptions::builder().force(true).build())
                    .await?;
                return Err(e.into());
            }
        };
        log::info!("Container finished {}", container.id());

        let mut logs = container.logs(
            &shiplift::LogsOptions::builder()
                .follow(false)
                .stdout(true)
                .stderr(true)
                .build(),
        );
        let mut stdout = String::new();
        let mut stderr = String::new();
        while let Some(chunk) = logs.next().await {
            match chunk? {
                TtyChunk::StdOut(bytes) => stdout.push_str(str::from_utf8(&bytes)?),
                TtyChunk::StdErr(bytes) => stderr.push_str(str::from_utf8(&bytes)?),
                TtyChunk::StdIn(_) => unreachable!(),
            }
        }

        container
            .remove(shiplift::RmContainerOptions::builder().force(true).build())
            .await?;
        log::info!("Container removed {}", container.id());

        Ok(Output {
            status: exit.status_code,
            stdout,
            stderr,
        })
    }
}

#[cfg(test)]
pub(crate) static TEST_RUNNER: once_cell::sync::Lazy<CodeRunner> =
    once_cell::sync::Lazy::new(|| CodeRunner {
        docker: Docker::new(),
        timeout: Duration::from_secs(10),
        // As much as needed
        cpus: 0.0,
        memory: 0,
        langs: HashMap::new(),
    });

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::*;

    #[tokio::test]
    async fn test_timeout() {
        assert!(matches!(
            TEST_RUNNER.run_code(&Python, "while True: pass").await,
            Err(_)
        ));
    }

    #[tokio::test]
    async fn test_output() {
        let code = r#"
import sys
sys.stdout.write('stdout')
sys.stderr.write('stderr')
sys.exit(123)
"#;
        let output = TEST_RUNNER.run_code(&Python, code).await.unwrap();
        assert_eq!(
            output,
            Output {
                status: 123,
                stdout: "stdout".into(),
                stderr: "stderr".into(),
            }
        );
    }
}
