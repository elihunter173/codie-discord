use std::path::Path;
use std::str;
use std::time::Duration;

use futures::StreamExt;

use shiplift::tty::TtyChunk;
use shiplift::Docker;

#[derive(Debug)]
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

pub struct Bot {
    docker: Docker,
    timeout: Duration,
    cpus: f64,
    memory: u64,
}

// TODO: Support options [[like this]]
impl Bot {
    pub async fn new(timeout: Duration, cpus: f64, memory: u64) -> Self {
        let docker = Docker::new();

        // I couldn't figure out lifetimes with the closure
        async fn pull_image(docker: &Docker, image: &str) {
            log::debug!("Pulling {}", image);
            let mut stream = docker
                .images()
                .pull(&shiplift::PullOptions::builder().image(image).build());
            while let Some(result) = stream.next().await {
                // TODO: Report status of pulling
                let data = result.unwrap();
                log::info!("{}: {:#?}", image, data);
            }
        }

        // Initializes all the docker images we use to ensure we never have to make someone wait to
        // run their code while we download the image
        log::info!("Pre-pulling all docker images");
        futures::future::join_all(Self::IMAGES.iter().map(|image| pull_image(&docker, image)))
            .await;
        log::info!("All docker images done pulling");

        Self {
            docker,
            timeout,
            cpus,
            memory,
        }
    }

    pub fn help(&self) -> &str {
        r#"Hi! I'm Codie the Code Runner.

I know how to run a variety of languages. All you have to do to ask me to run a block of code is to @ me in the message containing the code you want me to run. Make sure to include a language right after backticks (\`\`\`) or else I won't know how to run your code!

> @Codie Please run this code \`\`\`python
> print("Hi!")
> \`\`\`
"#
        // TODO: Commands
    }

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
                        .cpus(self.cpus)
                        .memory(self.memory)
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

macro_rules! count {
    ($x:literal) => (1);
    ($($xs:literal),*) => (0 $(+ count!($xs))*);
}

// TODO: Document the arguments
macro_rules! codegen_languages {
    ($(
    $name:ident = {
        display = $display:literal,
        codes = $($lang:literal)|+,
        image = $image:literal,
        cmd = $cmd:expr,
        path = $path:literal,
        help = $help:literal,
        hello_world = $hello_world:literal,
    },
    )*) => {

pub enum Language {
    $($name),*
}

impl Language {
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            $($($lang)|+ => Some(Self::$name),)*
            _ => None,
        }
    }
}

// HACK: In multiple places to display the language codes you see "some str" $(,"`",$lang,"`",)" /
// "+ "some str". This is to get around only being able to put a single token as the "join" token,
// in `concat!`. We'd like to say put backticks around every string and join them with the tokens
// [, " / " ,]. However, since we can't do that we take away the commas to the left and right of
// the reptition and bring them into the repeating part. That way we get something like (ignoring
// the backticks) `"LANGUAGE:" ,$lang, " / " ,$lang, "\n"

impl Bot {
    const IMAGES: [&'static str; count!($($image),*)] = [$($image),*];

    // TODO: Make this an option so error handling isn't done here and then we can get rid of the
    // user error type
    pub async fn run_code(&self, lang: Language, code: &str) -> anyhow::Result<Output> {
        match lang {
            $(Language::$name => self.run($image, &$cmd, $path, code).await,)*
        }
    }

    pub fn help_lang(&self, lang: Language) -> &str {
        match lang {
            $(Language::$name => concat!(
                "**LANGUAGE:** ", $display, " (" $(,"`",$lang,"`",)" / "+ ")\n",
                $help, "\n",
                "**EXAMPLE:** ```", $hello_world, "```",
            ),)*
        }
    }

    pub fn help_languages(&self) -> &str {
        concat!("I know\n", $("• ", $display, " (" $(,"`",$lang,"`",)" / "+ ")\n"),*)
    }
}

#[cfg(test)]
mod autogenerated_tests {
    use super::*;

    $(
        paste::item! {
            #[tokio::test]
            async fn [<test_hello_world_ $name:lower>]() {
                let output = RUNNER.run_code(Language::$name, $hello_world).await.unwrap();
                // Stderr first so we see errors
                assert_eq!(output.stderr, "");
                assert_eq!(output.stdout, "Hello, World!\n");
                assert_eq!(output.status, 0);
            }
        }
    )*

}

    }
}

codegen_languages! {
    Bash = {
        display = "Bash",
        codes = "bash",
        image = "bash:latest",
        cmd = ["bash", "run.sh"],
        path = "run.sh",
        help = "GNU's Bourne-again shell. The most common *NIX style shell today.",
        hello_world = "echo 'Hello, World!'",
    },
    C = {
        display = "C",
        codes = "c",
        image = "gcc:latest",
        cmd = ["sh", "-c", "gcc -Wall -Wextra main.c -o main && ./main"],
        path = "main.c",
        help = "",
        hello_world = r#"
#include<stdio.h>
int main() {
    printf("Hello, World!\n");
    return 0;
}"#,
    },
    Cpp = {
        display = "C++",
        codes = "cpp" | "c++",
        image = "gcc:latest",
        cmd = ["sh", "-c", "g++ -Wall -Wextra main.cpp -o main && ./main"],
        path = "main.cpp",
        help = "",
        hello_world = r#"
#include <iostream>
int main() {
    std::cout << "Hello, World!" << std::endl;
    return 0;
}"#,
    },
    Fortran = {
        display = "Fortran",
        codes = "fortran",
        image = "gcc:latest",
        cmd = ["sh", "-c", "gfortran -Wall -Wextra main.f95 -o main && ./main"],
        path = "main.f95",
        help = "",
        hello_world = r#"
program hello
  write(*,'(a)') "Hello, World!"
end program hello"#,
    },
    Go = {
        display = "Go",
        codes = "go",
        image = "golang:alpine",
        cmd = ["go", "run", "main.go"],
        path = "main.go",
        help = "",
        hello_world = r#"
package main
import "fmt"
func main() {
    fmt.Println("Hello, World!")
}"#,
    },
    Java = {
        display = "Java",
        codes = "java",
        image = "openjdk:alpine",
        cmd = [
            "sh",
            "-c",
            r"class=$(sed -n 's/public\s\+class\s\+\(\w\+\).*/\1/p' 0.java);
              ln -s 0.java $class.java && javac $class.java && java $class",
        ],
        // We pick a filename that can't be a classname
        path = "0.java",
        help = "",
        hello_world = r#"
public class Hello {
    public static void main(String[] args) {
        System.out.println("Hello, World!");
    }
}"#,
    },
    JavaScript = {
        display = "JavaScript",
        codes = "javascript" | "js",
        image = "node:alpine",
        cmd = ["node", "index.js"],
        path = "index.js",
        help = "",
        hello_world = "console.log('Hello, World!');",
    },
    Perl = {
        display = "Perl",
        codes = "perl",
        image = "perl:slim",
        cmd = ["perl", "main.pl"],
        path = "main.pl",
        help = "",
        hello_world = "print 'Hello, World!\n',",
    },
    Python = {
        display = "Python 3",
        codes = "python" | "py",
        image = "python:alpine",
        cmd = ["python", "main.py"],
        path = "main.py",
        help = "",
        hello_world = "print('Hello, World!')",
    },
    Ruby = {
        display = "Ruby",
        codes = "ruby",
        image = "ruby:alpine",
        cmd = ["ruby", "main.rb"],
        path = "main.rb",
        help = "",
        hello_world = "puts 'Hello, World!'",
    },
    Rust = {
        display = "Rust",
        codes = "rust",
        image = "rust:alpine",
        cmd = ["sh", "-c", "rustc main.rs && ./main"],
        path = "main.rs",
        help = "",
        hello_world = r#"
fn main() {
    println!("Hello, World!");
}"#,
    },
}

#[cfg(test)]
lazy_static::lazy_static! {
    static ref RUNNER: Bot = Bot {
        docker: Docker::new(),
        timeout: Duration::from_secs(10),
        // As much as needed
        cpus: 0.0,
        memory: 0,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_timeout() {
        assert!(matches!(
            RUNNER.run_code(Language::Python, "while True: pass").await,
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
        let output = RUNNER.run_code(Language::Python, code).await.unwrap();
        assert_eq!(output.stdout, "stdout");
        assert_eq!(output.stderr, "stderr");
        assert_eq!(output.status, 123);
    }
}
