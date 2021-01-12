use core::fmt;
use std::{borrow::Cow, collections::HashMap, path::Path, str, time::Duration};

use futures::{Stream, StreamExt};
use once_cell::sync::Lazy;
use regex::Regex;
use shiplift::{tty::TtyChunk, Docker};
use unicase::Ascii;

use crate::lang::LangRef;

#[derive(Debug, Eq, PartialEq)]
pub struct Output {
    pub status: u64,
    pub tty: Box<str>,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status == 0
    }
}

impl fmt::Display for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Replace ``` with something that looks really similar
        fn escape_codeblock(code: &str) -> Cow<str> {
            static CODE_BLOCK_FENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"```").unwrap());
            CODE_BLOCK_FENCE.replace_all(code, "\u{02CB}\u{02CB}\u{02CB}")
        }

        if !self.success() {
            write!(f, "**EXIT STATUS:** {}\n", self.status)?;
        }

        if !self.tty.is_empty() {
            // I like to keep self simple if there's no stderr
            write!(f, "```\n{}```", escape_codeblock(&self.tty))?;
        }
        Ok(())
    }
}

struct OutputBuilder<S>
where
    S: Stream<Item = shiplift::Result<TtyChunk>> + Unpin,
{
    buf: Vec<u8>,
    codepoints: usize,
    logs: Option<S>,
}

const MAX_OUTPUT_CODEPOINTS: usize = serenity::constants::MESSAGE_CODE_LIMIT as usize
    - "mentions_cost_22_chars: **EXIT STATUS:** 255\n```...```".len();

impl<S> OutputBuilder<S>
where
    S: Stream<Item = shiplift::Result<TtyChunk>> + Unpin,
{
    fn new(logs: S) -> Self {
        Self {
            buf: Vec::new(),
            codepoints: 0,
            logs: Some(logs),
        }
    }

    fn build(self) -> Box<str> {
        String::from_utf8(self.buf).unwrap().into_boxed_str()
    }

    async fn extend(&mut self) -> Result<(), ()> {
        let logs = match self.logs.as_mut() {
            Some(logs) => logs,
            None => return Err(()),
        };

        // TODO: Sometimes logs.next.await() == None even though not all the logs have been
        // returned... I think docker is closing our connection incorrectly?
        // To reproduce, run this until she outputs nothing
        // @Codie ```py
        // import sys
        // sys.stdout.write("x" * 1946)
        // ```
        while let Some(chunk) = logs.next().await {
            match chunk.unwrap() {
                TtyChunk::StdOut(ref mut bytes) | TtyChunk::StdErr(ref mut bytes) => {
                    // If we can count the codepoints, count them appropriately. If we can't,
                    // assume the worst case where each byte is a codepoint
                    self.codepoints += match str::from_utf8(bytes) {
                        Ok(s) => s.chars().count(),
                        Err(_) => bytes.len(),
                    };
                    if self.codepoints > MAX_OUTPUT_CODEPOINTS {
                        self.logs = None;
                        self.buf.extend_from_slice(b"...");
                        return Err(());
                    }
                    self.buf.append(bytes);
                }
                TtyChunk::StdIn(_) => unreachable!(),
            }
        }
        self.logs = None;
        Ok(())
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
                    // TODO: Restrict disk usage
                    &lang
                        .container_options()
                        // Run as user "nobody"
                        .user("65534:65534")
                        // Ensure that we are unprivileged
                        .capabilities(vec![])
                        .privileged(false)
                        // No internet access
                        .network_mode("none")
                        // Be in a safe directory
                        .working_dir("/tmp")
                        // Don't take too many resources
                        .cpus(self.cpus)
                        .memory(self.memory)
                        // Stop immediately
                        .stop_signal("SIGKILL")
                        .stop_timeout(Duration::from_nanos(0))
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

        async fn stop_container(container: &shiplift::Container<'_>) {
            match container.stop(Some(Duration::from_secs(0))).await {
                Ok(()) => {}
                // Means container is already stopped
                Err(shiplift::Error::Fault { code, .. }) if code == 304 => {}
                Err(err) => panic!(err),
            }
        }
        let mut output_builder = OutputBuilder::new(
            container.logs(
                &shiplift::LogsOptions::builder()
                    .follow(true)
                    .stdout(true)
                    .stderr(true)
                    .build(),
            ),
        );
        let run_fut = tokio::time::timeout(self.timeout, async {
            if output_builder.extend().await.is_err() {
                return Err(());
            }
            let exit = container.wait().await.unwrap();
            Ok(exit)
        });
        let exit = match run_fut.await {
            // Finished successfully within time
            Ok(Ok(exit)) => {
                log::info!("Container finished {}", container.id());
                exit
            }
            Ok(Err(_overflowed)) => {
                log::warn!(
                    "Force stopping container {}. Reason: overflowed output",
                    container.id()
                );
                stop_container(&container).await;
                container.wait().await?
            }
            // Timed out
            Err(_elapsed) => {
                log::warn!(
                    "Force stopping container {}. Reason: exceeded timeout",
                    container.id()
                );
                stop_container(&container).await;
                container.wait().await?
            }
        };

        // We may have timed out earlier and have some logs left over. Since the container has
        // stopped, we can safely try to get all remaining logs without missing any.
        let _ = output_builder.extend().await;

        container
            .remove(shiplift::RmContainerOptions::builder().force(true).build())
            .await?;
        log::info!("Container removed {}", container.id());
        Ok(Output {
            status: exit.status_code,
            tty: output_builder.build(),
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
        let output = TEST_RUNNER
            .run_code(&Python, "while True: pass")
            .await
            .unwrap();
        assert_eq!(
            output,
            Output {
                // The status Python returns from SIGKILL
                status: 137,
                tty: "".into(),
            }
        );
    }

    #[tokio::test]
    async fn test_output() {
        let code = r#"
import sys
print('stdout', file=sys.stdout)
print('stderr', file=sys.stderr)
sys.exit(123)
"#;
        let output = TEST_RUNNER.run_code(&Python, code).await.unwrap();
        assert_eq!(
            output,
            Output {
                status: 123,
                tty: "stdout\nstderr\n".into(),
            }
        );
    }

    #[tokio::test]
    async fn test_ordering() {
        // This code causes issues with python buffering sometimes...
        let code = r#"
import os
print(0)
os.system("echo 1")
print(2)
"#;
        let output = TEST_RUNNER.run_code(&Python, code).await.unwrap();
        assert_eq!(
            output,
            Output {
                status: 0,
                tty: "0\n1\n2\n".into(),
            }
        );
    }
}
