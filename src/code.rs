use std::path::Path;
use std::str;
use std::time::Duration;

use futures::StreamExt;

use shiplift::tty::TtyChunk;
use shiplift::Docker;

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
    docker: Docker,
    timeout: Duration,
}

impl CodeRunner {
    pub async fn with_timeout(timeout: Duration) -> Self {
        let docker = Docker::new();

        // Initializes all the docker images we use to ensure we never have to make someone wait to
        // run their code while we download the image
        log::info!("Pre-pulling all docker images");
        // TODO: Maybe don't hardcode these
        let images = vec![
            "bash:latest",
            "python:alpine",
            "ruby:alpine",
            "node:alpine",
            "perl:slim",
            "rust:alpine",
            "gcc:latest",
            "golang:alpine",
            "openjdk:alpine",
        ];
        futures::future::join_all(images.iter().map(|image| pull_image(&docker, image))).await;
        log::info!("All docker images done pulling");

        Self { timeout, docker }
    }
}

async fn pull_image(docker: &Docker, image: &str) {
    log::debug!("Pulling {}", image);
    docker
        .images()
        .pull(&shiplift::PullOptions::builder().image(image).build())
        .next()
        .await
        .unwrap()
        .unwrap();
}

// TODO: Support options [[like this]]

impl CodeRunner {
    pub async fn run_code(&self, lang: &str, code: &str) -> anyhow::Result<Output> {
        match lang {
            "bash" => {
                self.run("bash:latest", &["bash", "main.bash"], "main.bash", code)
                    .await
            }
            "python" => {
                self.run("python:alpine", &["python", "main.py"], "main.py", code)
                    .await
            }
            "ruby" => {
                self.run("ruby:alpine", &["ruby", "main.rb"], "main.rb", code)
                    .await
            }
            "javascript" => {
                self.run("node:alpine", &["node", "main.js"], "main.js", code)
                    .await
            }
            "perl" => {
                self.run("perl:slim", &["perl", "main.pl"], "main.pl", code)
                    .await
            }
            "rust" => {
                self.run(
                    "rust:alpine",
                    &["sh", "-c", "rustc main.rs -o exe && ./exe"],
                    "main.rs",
                    code,
                )
                .await
            }
            "c" => {
                self.run(
                    "gcc:latest",
                    &["sh", "-c", "gcc -Wall -Wextra main.c -o exe && ./exe"],
                    "main.c",
                    code,
                )
                .await
            }
            "c++" | "cpp" => {
                self.run(
                    "gcc:latest",
                    &["sh", "-c", "g++ -Wall -Wextra main.cpp -o exe && ./exe"],
                    "main.cpp",
                    code,
                )
                .await
            }
            "go" => {
                self.run("golang:alpine", &["go", "run", "main.go"], "main.go", code)
                    .await
            }
            "java" => {
                self.run(
                    "openjdk:alpine",
                    &[
                        "sh",
                        "-c",
                        r"
                        class=$(sed -n 's/public\s\+class\s\+\(\w\+\).*/\1/p' main.java);
                        ln -s main.java $class.java && javac $class.java && java $class",
                    ],
                    "main.java",
                    code,
                )
                .await
            }
            _ => {
                Err(crate::UserError(format!("I'm sorry, I don't know how to run {}", lang)).into())
            }
        }
    }

    // TODO: Use shiplift everywhere below here
    async fn run(
        &self,
        image: &str,
        command: &[&str],
        path: impl AsRef<Path>,
        code: &str,
    ) -> anyhow::Result<Output> {
        let container = {
            let response = self
                .docker
                .containers()
                .create(
                    &shiplift::ContainerOptions::builder(image)
                        .attach_stdout(true)
                        .attach_stderr(true)
                        // Run as user "nobody"
                        .user("65534:65534")
                        // TODO: This is a hack. I should make this specific to each container
                        .env(vec!["GOCACHE=/tmp/.cache/go"])
                        // Ensure that we run unprivileged
                        .capabilities(vec![])
                        // No internet access
                        .network_mode("none")
                        // Make it so you can just open files
                        .working_dir("/tmp")
                        .cmd(Vec::from(command))
                        .build(),
                )
                .await?;
            shiplift::Container::new(&self.docker, response.id)
        };
        container
            .copy_file_into(Path::new("/tmp").join(path), code.as_bytes())
            .await?;

        log::info!("Starting container {}", container.id());
        container.start().await?;
        let exit = match tokio::time::timeout(self.timeout, container.wait()).await {
            Ok(result) => result?,
            Err(e) => {
                log::warn!(
                    "Killing container {}. Reason: exceeded timeout",
                    container.id()
                );
                container.kill(None).await?;
                container
                    .remove(shiplift::RmContainerOptions::builder().build())
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
            .remove(shiplift::RmContainerOptions::builder().build())
            .await?;
        log::info!("Container removed {}", container.id());

        Ok(Output {
            status: exit.status_code,
            stdout,
            stderr,
        })
    }
}
