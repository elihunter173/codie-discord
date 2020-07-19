use std::time::Duration;

use std::process::{Output, Stdio};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use anyhow;
use log;

pub struct CodeRunner {
    timeout: Duration,
}

impl CodeRunner {
    /// timeout is in seconds
    // TODO: Make it more clear what 30 is. It's timeout in seconds. Maybe with_timeout?
    pub async fn with_timeout(timeout: Duration) -> Self {
        // Initializes all the docker images we use to ensure we never have to make someone wait to
        // run their code while we download the image
        log::info!("Pre-pulling all docker images");
        // TODO: Maybe don't hardcode these
        let images = vec![
            "bash:latest",
            "python:alpine",
            "ruby:alpine",
            "node:alpine",
            "perl:alpine",
            "rust:alpine",
            "gcc:latest",
            "golang:alpine",
            "openjdk:alpine",
        ];
        // TODO: Ensure that all commands exit successfully and that the futures run successfully
        futures::future::join_all(images.iter().map(|image| {
            Command::new("docker")
                .arg("pull")
                .arg(image)
                .kill_on_drop(true)
                .status()
        }))
        .await;
        log::info!("All docker images done pulling");

        Self { timeout }
    }
}

// TODO: Support options [[like this]]
// TODO: Use shiplift instead of the docker command line. Needs to support attaching first tho

impl CodeRunner {
    pub async fn run_code(&self, lang: &str, code: &str) -> anyhow::Result<Output> {
        match lang {
            "bash" => self.run("bash:latest", &["bash"], code).await,
            "python" => self.run("python:alpine", &["python"], code).await,
            "ruby" => self.run("ruby:alpine", &["ruby"], code).await,
            "javascript" => self.run("node:alpine", &["node"], code).await,
            "perl" => self.run("perl:alpine", &["perl"], code).await,
            "rust" => {
                self.run(
                    "rust:alpine",
                    &["sh", "-c", "rustc /dev/stdin -o exe && ./exe"],
                    code,
                )
                .await
            }
            "c" => {
                self.run(
                    "gcc:latest",
                    &[
                        "sh",
                        "-c",
                        "gcc -Wall -Wextra -x c /dev/stdin -o exe && ./exe",
                    ],
                    code,
                )
                .await
            }
            "c++" | "cpp" => {
                self.run(
                    "gcc:latest",
                    &[
                        "sh",
                        "-c",
                        "g++ -Wall -Wextra -x c++ /dev/stdin -o exe && ./exe",
                    ],
                    code,
                )
                .await
            }
            "go" => {
                self.run(
                    "golang:alpine",
                    &[
                        "sh",
                        "-c",
                        // Go treats all files that don't end in .go as packages and I can't figure out how to
                        // work around that
                        "cat /dev/stdin > main.go && go run main.go",
                    ],
                    code,
                )
                .await
            }
            "java" => {
                self.run(
                    "openjdk:alpine",
                    &[
                        "sh",
                        "-c",
                        "(echo 'public class Exe {' \
                            && cat /dev/stdin \
                            && echo '}') > Exe.java \
                        && javac Exe.java \
                        && java Exe",
                    ],
                    code,
                )
                .await
            }
            _ => Err(crate::UserError(format!("unknown language: {}", lang)).into()),
        }
    }

    async fn run(&self, image: &str, command: &[&str], stdin: &str) -> anyhow::Result<Output> {
        tokio::time::timeout(self.timeout, self.run_no_timeout(image, command, stdin)).await?
    }

    // TODO: Fix killing the container. This kill_on_drop still leaves docker container running.
    // This requires using shiplift I believe
    async fn run_no_timeout(
        &self,
        image: &str,
        command: &[&str],
        stdin: &str,
    ) -> anyhow::Result<Output> {
        let mut child = Command::new("docker")
            .args(&[
                "run",
                // Connect stdin, stdout, and stderr
                "--interactive",
                // Remove container after it stops
                "--rm",
                // No internet access
                "--network=none",
                // Ensure that we run unpriveleged
                "--cap-drop=ALL",
                // Run as user "nobody"
                "--user=65534:65534",
                // Make it so you can just open files
                "--workdir=/tmp",
            ])
            .arg(image)
            .args(command)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write(stdin.as_bytes())
            .await?;
        Ok(child.wait_with_output().await?)
    }
}
