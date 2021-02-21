use core::fmt;
use std::{borrow::Cow, collections::HashMap, str, time::Duration};

use futures::{Stream, StreamExt};
use once_cell::sync::Lazy;
use regex::Regex;
use shiplift::{tty::TtyChunk, Docker};
use tokio::{fs::File, io::AsyncWriteExt};
use unicase::Ascii;

use crate::{lang::LangRef, logging::Loggable};

#[derive(Debug)]
pub struct UnrecognizedContainer;

impl fmt::Display for UnrecognizedContainer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for UnrecognizedContainer {}

pub struct RunSpec {
    pub code_path: &'static str,
    pub image_name: String,
    pub dockerfile: String,
}

pub struct CodeRunner {
    pub docker: Docker,
    pub langs: HashMap<Ascii<&'static str>, LangRef>,
    pub timeout: Duration,
    pub cpus: f64,
    pub memory: u64,
}

impl CodeRunner {
    pub fn get_lang_by_code(&self, code: &str) -> Option<LangRef> {
        self.langs.get(&Ascii::new(code)).copied()
    }

    pub async fn build<'s>(&'s self, spec: &'s RunSpec) -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;

        let file_path = dir.path().join("Dockerfile");
        let mut file = File::create(file_path).await?;
        file.write_all(spec.dockerfile.as_bytes()).await?;
        file.flush().await?;

        let dir_str = dir.path().to_str().unwrap();

        let image_name = format!("codie/{}", spec.image_name);
        log::info!("Building {}", image_name);
        let images = self.docker.images();
        let build_opts = shiplift::BuildOptions::builder(dir_str)
            .tag(image_name)
            .build();
        let mut stream = images.build(&build_opts);
        while let Some(build_result) = stream.next().await {
            match build_result {
                Ok(output) => match output.get("error") {
                    Some(_) => anyhow::bail!("build error: {:?}", output),
                    None => log::debug!("{:?}", output),
                },
                Err(e) => anyhow::bail!("failed while building: {:?}", e),
            }
        }
        Ok(())
    }

    pub async fn run_code<'s>(
        &'s self,
        spec: &'s RunSpec,
        code: &'s str,
    ) -> anyhow::Result<Output> {
        // TODO: Restrict disk usage
        let container_opts =
            shiplift::ContainerOptions::builder(&format!("codie/{}", &spec.image_name))
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
                .build();
        let container = match self.docker.containers().create(&container_opts).await {
            Ok(response) => shiplift::Container::new(&self.docker, response.id),
            Err(shiplift::Error::Fault { code, .. }) if code == 404 => {
                return Err(UnrecognizedContainer.into());
            }
            Err(err) => return Err(err.into()),
        };
        container
            .copy_file_into(spec.code_path, code.as_bytes())
            .await?;

        log::info!("{} starting", container.as_log());
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
                log::info!("{} finished", container.as_log());
                exit
            }
            Ok(Err(_overflowed)) => {
                log::warn!(
                    "{} force-stopping. Reason: overflowed output",
                    container.as_log()
                );
                stop_container(&container).await;
                container.wait().await?
            }
            // Timed out
            Err(_elapsed) => {
                log::warn!(
                    "{} force-stopping. Reason: exceeded timeout",
                    container.as_log()
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
        log::info!("{} removed", container.as_log());
        Ok(Output {
            status: exit.status_code,
            tty: output_builder.build(),
        })
    }
}

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

        write!(f, "```\n{}```", escape_codeblock(&self.tty))?;
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
        // returned... I think docker is closing our connection incorrectly? See `test_no_newline`
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

#[cfg(test)]
pub(crate) async fn test_run<'s>(lang: LangRef, code: &'s str) -> anyhow::Result<Output> {
    static TEST_RUNNER: once_cell::sync::Lazy<CodeRunner> =
        once_cell::sync::Lazy::new(|| CodeRunner {
            docker: Docker::new(),
            timeout: Duration::from_secs(10),
            // As much as needed
            cpus: 0.0,
            memory: 0,
            langs: HashMap::new(),
        });

    let spec = lang.run_spec(Default::default()).unwrap();
    match TEST_RUNNER.run_code(&spec, code).await {
        Ok(output) => Ok(output),
        Err(err) => match err.downcast_ref::<UnrecognizedContainer>() {
            Some(_) => {
                TEST_RUNNER.build(&spec).await.unwrap();
                TEST_RUNNER.run_code(&spec, code).await
            }
            None => Err(err),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::*;

    #[tokio::test]
    async fn test_timeout() {
        let output = test_run(&Python, "while True: pass").await.unwrap();
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
        let output = test_run(&Python, code).await.unwrap();
        assert_eq!(
            output,
            Output {
                status: 123,
                tty: "stdout\nstderr\n".into(),
            }
        );
    }

    // TODO: This test is flaky...
    #[tokio::test]
    async fn test_ordering() {
        // This code causes issues with python buffering sometimes
        let code = r#"
import os
print(0)
os.system("echo 1")
print(2)
"#;
        let output = test_run(&Python, code).await.unwrap();
        assert_eq!(
            output,
            Output {
                status: 0,
                tty: "0\n1\n2\n".into(),
            }
        );
    }

    // TODO: This test is flaky...
    #[tokio::test]
    async fn test_no_newline() {
        let code = r#"
import sys
sys.stdout.write("x" * 1000)
"#;
        let output = test_run(&Python, code).await.unwrap();
        assert_eq!(
            output,
            Output {
                status: 0,
                tty: "x".repeat(1000).into(),
            }
        );
    }
}
