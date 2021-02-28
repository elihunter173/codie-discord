use std::fmt;

use thiserror::Error;
use unicase::Ascii;

use crate::{options_parser::Options, runner::RunSpec};

#[derive(Error, Debug)]
pub enum OptionsError {
    #[error("unrecognized key `{0:?}`")]
    UnknownKeys(Vec<String>),
    #[error("unrecognized values `{0:?}`")]
    UnknownValue(String),
}

pub trait Language: fmt::Display {
    // From https://github.com/highlightjs/highlight.js/blob/master/SUPPORTED_LANGUAGES.md.
    fn codes(&self) -> &[Ascii<&str>];
    fn run_spec(&self, opts: Options) -> anyhow::Result<RunSpec, OptionsError>;
}

pub type LangRef = &'static (dyn Language + Send + Sync);
inventory::collect!(LangRef);

macro_rules! make_lang {
    ($lang:ident, $($name:tt)+) => {
        pub struct $lang;
        inventory::submit!(&$lang as LangRef);
        impl fmt::Display for $lang {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, $($name)*)
            }
        }
    };
    ($lang:ident) => {
        make_lang!($lang, stringify!($lang));
    };
}

macro_rules! test_lang {
    ($lang:ident, $code:literal) => {
        #[cfg(test)]
        paste::paste! {
            #[tokio::test]
            async fn [<test_hello_world_ $lang:lower>]() {
                let output = $crate::runner::test_run(&$lang, $code).await.unwrap();
                assert_eq!(
                    output,
                    $crate::runner::Output {
                        status: 0,
                        tty: "Hello, World!\n".into(),
                    }
                );
            }
        }
    };
}

macro_rules! count {
    ($x:tt) => (1);
    ($($xs:tt),*) => (0 $(+ count!($xs))*);
}

macro_rules! codes {
    ($($codes:literal),*) => (
        {
            // We declare a const and then take a ref to it because we want a 'static slice. If we
            // just directly took a ref then it would have a temporary lifetime
            const CODES: [Ascii<&str>; count!($($codes),*)] = [$(Ascii::new($codes)),*];
            &CODES
        }
    )
}

macro_rules! bind_opts {
    ( $map:expr => {$( $vars:ident $(or $default:literal)? ),*$(,)?} ) => (
        #[allow(unused_parens, unused_mut)]
        let ($($vars),*) = {
            let mut m = $map;
            let tup = ($( m.remove(stringify!($vars)) $(.unwrap_or(String::from($default)))? ),*);
            if !m.is_empty() {
                return Err(OptionsError::UnknownKeys(m.keys().map(|&s| s.to_owned()).collect()));
            }
            tup
        };
    )
}

make_lang!(Sh);
impl Language for Sh {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["sh"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => {});
        Ok(RunSpec {
            image_name: "sh".to_owned(),
            code_path: "/tmp/run.sh",
            dockerfile: "
FROM alpine:3.13
CMD sh /tmp/run.sh
"
            .to_owned(),
        })
    }
}
test_lang!(Sh, "echo 'Hello, World!'");

make_lang!(Bash);
impl Language for Bash {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["bash"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => {});
        Ok(RunSpec {
            image_name: "bash".to_owned(),
            code_path: "/tmp/run.sh",
            dockerfile: "
FROM alpine:3.13
RUN apk add --no-cache bash
CMD bash /tmp/run.sh
"
            .to_owned(),
        })
    }
}
test_lang!(Bash, "echo 'Hello, World!'");

make_lang!(Zsh);
impl Language for Zsh {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["zsh"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => {});
        Ok(RunSpec {
            image_name: "zsh".to_owned(),
            code_path: "/tmp/run.sh",
            dockerfile: "
FROM alpine:3.13
RUN apk add --no-cache zsh
CMD zsh /tmp/run.sh
"
            .to_owned(),
        })
    }
}
test_lang!(Zsh, "echo 'Hello, World!'");

make_lang!(Python);
impl Language for Python {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["python", "py", "gyp"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => { version or "3.9", bundle or "scipy" });
        match version.as_str() {
            "3.9" | "3.8" | "3.7" | "3.6" => (),
            _ => return Err(OptionsError::UnknownValue(version)),
        };
        let pip_install = match bundle.as_str() {
            "none" => "",
            "scipy" => "RUN pip install numpy scipy sympy",
            _ => return Err(OptionsError::UnknownValue(bundle)),
        };

        Ok(RunSpec {
            image_name: format!("python{}-{}", version, bundle),
            code_path: "/tmp/run.py",
            dockerfile: format!(
                "
FROM python:{version}-slim-buster
ENV PYTHONUNBUFFERED=1
{pip_install}
CMD python /tmp/run.py
",
                version = version,
                pip_install = pip_install,
            ),
        })
    }
}
test_lang!(Python, "print('Hello, World!')");

make_lang!(JavaScript);
impl Language for JavaScript {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["javascript", "js", "jsx"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => { version or "15" });
        match version.as_str() {
            "15" | "14" | "12" | "10" => (),
            _ => return Err(OptionsError::UnknownValue(version)),
        };
        Ok(RunSpec {
            image_name: format!("nodejs{}", version),
            code_path: "/tmp/index.js",
            dockerfile: format!(
                "
FROM node:{version}-alpine
CMD node /tmp/index.js
",
                version = version,
            ),
        })
    }
}
test_lang!(JavaScript, "console.log('Hello, World!');");

make_lang!(TypeScript);
impl Language for TypeScript {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["typescript", "ts"]
    }
    fn run_spec(&self, opts: Options) -> anyhow::Result<RunSpec, OptionsError> {
        bind_opts!(opts => {});
        Ok(RunSpec {
            image_name: "typescript".to_owned(),
            code_path: "/tmp/index.ts",
            // This is taken from https://github.com/hayd/deno-docker/blob/master/distroless.dockerfile
            dockerfile: r#"
FROM alpine:3.12.3

ENV DENO_VERSION=1.7.2

RUN apk add --virtual .download --no-cache curl \
 && curl -fsSL https://github.com/denoland/deno/releases/download/v${DENO_VERSION}/deno-x86_64-unknown-linux-gnu.zip \
         --output deno.zip \
 && unzip deno.zip \
 && rm deno.zip \
 && chmod 755 deno \
 && mv deno /bin/deno \
 && apk del .download


FROM gcr.io/distroless/cc
COPY --from=0 /bin/deno /bin/deno

ENV DENO_VERSION=1.7.2
ENV DENO_DIR /tmp/deno
ENV DENO_INSTALL_ROOT /usr/local
CMD ["/bin/deno", "run", "--quiet", "/tmp/index.ts"]
"#
            .to_owned(),
        })
    }
}
test_lang!(TypeScript, "console.log('Hello, World!');");

make_lang!(Perl);
impl Language for Perl {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["perl", "pl", "pm"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => {});
        Ok(RunSpec {
            image_name: "perl".to_owned(),
            code_path: "/tmp/run.pl",
            dockerfile: "
FROM perl:slim-buster
CMD perl /tmp/run.pl
"
            .to_owned(),
        })
    }
}
test_lang!(Perl, "print 'Hello, World!\n'");

make_lang!(PHP);
impl Language for PHP {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["php", "php3", "php4", "php5", "php6", "php7", "php8"]
    }
    fn run_spec(&self, opts: Options) -> anyhow::Result<RunSpec, OptionsError> {
        bind_opts!(opts => {});
        Ok(RunSpec {
            image_name: "php".to_owned(),
            code_path: "/tmp/run.php",
            dockerfile: r#"
FROM php:8.0-alpine
CMD ["php", "run.php"]
"#
            .to_owned(),
        })
    }
}
test_lang!(PHP, "<?php echo 'Hello, World!\n' ?>");

make_lang!(Ruby);
impl Language for Ruby {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["ruby", "rb", "gemspec", "podspec", "thor", "irb"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        // TODO: Support JRuby
        bind_opts!(opts => { version or "3.0" });
        match version.as_str() {
            "3.0" | "2.7" | "2.6" | "2.5" => (),
            _ => return Err(OptionsError::UnknownValue(version)),
        };

        Ok(RunSpec {
            image_name: format!("ruby{}", version),
            code_path: "/tmp/run.rb",
            dockerfile: format!(
                "
FROM ruby:{version}-alpine
CMD ruby /tmp/run.rb
",
                version = version
            ),
        })
    }
}
test_lang!(Ruby, "puts 'Hello, World!'");

make_lang!(Lua);
impl Language for Lua {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["lua"]
    }
    fn run_spec(&self, opts: Options) -> anyhow::Result<RunSpec, OptionsError> {
        bind_opts!(opts => { version or "5.4" });
        // TODO: Add LuaJIT
        match version.as_str() {
            "5.4" | "5.3" | "5.2" | "5.1" => (),
            _ => return Err(OptionsError::UnknownValue(version)),
        };
        Ok(RunSpec {
            image_name: format!("lua{}", version),
            code_path: "/tmp/run.lua",
            dockerfile: format!(
                "
FROM alpine:edge
RUN apk add --no-cache lua{version}
CMD lua{version} /tmp/run.lua
",
                version = version
            ),
        })
    }
}
test_lang!(Lua, "print('Hello, World!')");

make_lang!(Julia);
impl Language for Julia {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["julia", "julia-repl"]
    }
    fn run_spec(&self, opts: Options) -> anyhow::Result<RunSpec, OptionsError> {
        bind_opts!(opts => { version or "1.6" });
        match version.as_str() {
            "1.6" | "1.5" | "1.0" => (),
            _ => return Err(OptionsError::UnknownValue(version)),
        };
        Ok(RunSpec {
            image_name: format!("julia{}", version),
            code_path: "/tmp/run.jl",
            dockerfile: format!(
                "
FROM julia:{version}
CMD julia /tmp/run.jl
",
                version = version
            ),
        })
    }
}
test_lang!(Julia, "println(\"Hello, World!\")");

make_lang!(Go);
impl Language for Go {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["go", "golang"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => { version or "1.16" });
        match version.as_str() {
            "1.16" | "1.15" => (),
            _ => return Err(OptionsError::UnknownValue(version)),
        };

        Ok(RunSpec {
            image_name: format!("golang{}", version),
            code_path: "/tmp/main.go",
            dockerfile: format!(
                "
FROM golang:{version}-alpine
# So that we can build code
ENV GOCACHE=/tmp/.cache/go
CMD go run /tmp/main.go
",
                version = version
            ),
        })
    }
}
test_lang!(
    Go,
    r#"
package main
import "fmt"
func main() {
    fmt.Println("Hello, World!")
}"#
);

make_lang!(Java);
impl Language for Java {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["java", "jsp"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => { version or "15" });
        match version.as_str() {
            "17" | "16" | "15" | "11" | "8" => (),
            _ => return Err(OptionsError::UnknownValue(version)),
        };
        Ok(RunSpec {
            image_name: format!("openjdk{}", version),
            code_path: "/tmp/code",
            dockerfile: format!(
                r#"
FROM openjdk:{version}-jdk-slim-buster
# The sed command grabs the classname from `public class Ident`
CMD sh -c \
    'class=$(sed -n "s/public\s\+class\s\+\(\w\+\).*/\1/p" code); \
     ln -s code $class.java && javac $class.java && java $class'
"#,
                version = version
            ),
        })
    }
}
test_lang!(
    Java,
    r#"
public class Hello {
    public static void main(String[] args) {
        System.out.println("Hello, World!");
    }
}"#
);

make_lang!(CSharp, "C#");
impl Language for CSharp {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["csharp", "cs"]
    }
    fn run_spec(&self, opts: Options) -> anyhow::Result<RunSpec, OptionsError> {
        bind_opts!(opts => {});
        Ok(RunSpec {
            image_name: "csharp".to_owned(),
            code_path: "/tmp/main.cs",
            dockerfile: r#"
FROM mono:6.12
CMD ["sh", "-c", "mcs -out:main.exe main.cs && mono main.exe" ]
"#
            .to_owned(),
        })
    }
}
test_lang!(
    CSharp,
    r#"
class HelloWorld {
    static void Main() {
        System.Console.WriteLine("Hello, World!");
    }
}"#
);

make_lang!(Swift);
impl Language for Swift {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["swift"]
    }
    fn run_spec(&self, opts: Options) -> anyhow::Result<RunSpec, OptionsError> {
        bind_opts!(opts => { version or "5.3" });
        match version.as_str() {
            "5.3" | "5.2" | "5.1" => (),
            _ => return Err(OptionsError::UnknownValue(version)),
        };
        Ok(RunSpec {
            image_name: format!("swift{}", version),
            code_path: "/tmp/main.swift",
            dockerfile: format!(
                r#"
FROM swift:{version}
CMD ["swift", "main.swift" ]
"#,
                version = version,
            ),
        })
    }
}
test_lang!(Swift, "print(\"Hello, World!\")");

make_lang!(C);
impl Language for C {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["c", "h"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => {});
        // TODO: Support clang, CFLAGS, and different versions of gcc

        Ok(RunSpec {
            image_name: "c-gcc".to_owned(),
            code_path: "/tmp/main.c",
            dockerfile: "
FROM gcc:latest
CMD sh -c 'gcc -Wall -Wextra main.c -o main && ./main'
"
            .to_owned(),
        })
    }
}
test_lang!(
    C,
    r#"
#include <stdio.h>
int main() {
    printf("Hello, World!\n");
    return 0;
}"#
);

make_lang!(Cpp, "C++");
impl Language for Cpp {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["cpp", "hpp", "cc", "hh", "c++", "h++", "cxx", "hxx"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => {});
        // TODO: Support clang, CFLAGS, and different versions of gcc

        Ok(RunSpec {
            image_name: "cpp-gcc".to_owned(),
            code_path: "/tmp/main.cpp",
            dockerfile: "
FROM gcc:latest
CMD sh -c 'g++ -Wall -Wextra main.cpp -o main && ./main'
"
            .to_owned(),
        })
    }
}
test_lang!(
    Cpp,
    r#"
    #include <iostream>
int main() {
    std::cout << "Hello, World!" << std::endl;
    return 0;
}"#
);

make_lang!(Rust);
impl Language for Rust {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["rust", "rs"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => {});
        // TODO: Support rust versions and nightly features

        Ok(RunSpec {
            image_name: "rust".to_owned(),
            code_path: "/tmp/main.rs",
            dockerfile: "
FROM rust:alpine
CMD sh -c 'rustc main.rs -o main && ./main'
"
            .to_owned(),
        })
    }
}
test_lang!(
    Rust,
    r#"
fn main() {
    println!("Hello, World!");
}"#
);

make_lang!(Fortran);
impl Language for Fortran {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["fortran", "f90", "f95"]
    }
    fn run_spec(&self, opts: Options) -> Result<RunSpec, OptionsError> {
        bind_opts!(opts => {});

        Ok(RunSpec {
            image_name: "fortran".to_owned(),
            code_path: "/tmp/main.f95",
            dockerfile: "
FROM gcc:latest
CMD sh -c 'gfortran -Wall -Wextra main.f95 -o main && ./main'
"
            .to_owned(),
        })
    }
}
test_lang!(
    Fortran,
    r#"
program hello
    write(*,'(a)') "Hello, World!"
end program hello"#
);
