use std::fmt::Display;

use shiplift::{builder::ContainerOptionsBuilder, ContainerOptions};
use unicase::Ascii;

pub trait Language: Display {
    // const HELP: &'static str;
    // const DISPLAY: &'static str;
    // Used to notify
    // const FILENAME: &'static str;
    // const HELLO_WORLD: &'static str;
    // const CODES: &'static [&'static str];
    // From https://github.com/highlightjs/highlight.js/blob/master/SUPPORTED_LANGUAGES.md.
    fn codes(&self) -> &[Ascii<&str>];
    fn container_options(&self) -> ContainerOptionsBuilder;
}

pub type LangRef = &'static (dyn Language + Send + Sync);
inventory::collect!(LangRef);

macro_rules! make_lang {
    ($lang:ident) => {
        pub struct $lang;
        inventory::submit!(&$lang as LangRef);
        impl Display for $lang {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, stringify!($lang))
            }
        }
    };
}

macro_rules! test_lang {
    ($lang:ident, $code:literal) => {
        #[cfg(test)]
        paste::paste! {
            #[tokio::test]
            async fn [<test_hello_world_ $lang:lower>]() {
                let output = $crate::runner::TEST_RUNNER.run_code(&$lang, $code).await.unwrap();
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
    ($x:literal) => (1);
    ($($xs:literal),*) => (0 $(+ count!($xs))*);
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

make_lang!(Bash);
impl Language for Bash {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["bash", "sh", "zsh"]
    }
    fn container_options(&self) -> ContainerOptionsBuilder {
        let mut builder = ContainerOptions::builder("bash");
        builder.cmd(vec!["bash", "code"]);
        builder
    }
}
test_lang!(Bash, "echo 'Hello, World!'");

make_lang!(C);
impl Language for C {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["c", "h"]
    }
    fn container_options(&self) -> ContainerOptionsBuilder {
        let mut builder = ContainerOptions::builder("gcc");
        builder.cmd(vec![
            "sh",
            "-c",
            "gcc -Wall -Wextra -x c code -o exe && ./exe",
        ]);
        builder
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

make_lang!(Cpp);
impl Language for Cpp {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["cpp", "hpp", "cc", "hh", "c++", "h++", "cxx", "hxx"]
    }
    fn container_options(&self) -> ContainerOptionsBuilder {
        let mut builder = ContainerOptions::builder("gcc");
        builder.cmd(vec![
            "sh",
            "-c",
            "g++ -Wall -Wextra -x c++ code -o exe && ./exe",
        ]);
        builder
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

make_lang!(Fortran);
impl Language for Fortran {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["fortran", "f90", "f95"]
    }
    fn container_options(&self) -> ContainerOptionsBuilder {
        let mut builder = ContainerOptions::builder("gcc");
        builder.cmd(vec![
            "sh",
            "-c",
            // Fortran has a free-form vs fixed-form property for source code:
            // https://people.cs.vt.edu/~asandu/Courses/MTU/CS2911/fortran_notes/node4.html
            // free-form is the default for f90 and above. Normally it is picked by the extension.
            // However, we don't have an extension here so we have to manually specify it with
            // -ffree-form
            "gfortran -Wall -Wextra -x f95 -ffree-form code -o exe && ./exe",
        ]);
        builder
    }
}
test_lang!(
    Fortran,
    r#"
program hello
    write(*,'(a)') "Hello, World!"
end program hello"#
);

make_lang!(Go);
impl Language for Go {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["go", "golang"]
    }
    fn container_options(&self) -> ContainerOptionsBuilder {
        let mut builder = ContainerOptions::builder("golang:alpine");
        builder
            .cmd(vec!["sh", "-c", "ln -s code code.go && go run code.go"])
            .env(["GOCACHE=/tmp/.cache/go"]);
        builder
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
    fn container_options(&self) -> ContainerOptionsBuilder {
        let mut builder = ContainerOptions::builder("openjdk:alpine");
        builder.cmd(vec![
            "sh",
            "-c",
            // Grabs the classname from `public class Ident`
            r"class=$(sed -n 's/public\s\+class\s\+\(\w\+\).*/\1/p' code);
                  ln -s code $class.java && javac $class.java && java $class",
        ]);
        builder
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

make_lang!(JavaScript);
impl Language for JavaScript {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["javascript", "js", "jsx"]
    }
    fn container_options(&self) -> ContainerOptionsBuilder {
        let mut builder = ContainerOptions::builder("node:alpine");
        builder.cmd(vec!["node", "code"]);
        builder
    }
}
test_lang!(JavaScript, "console.log('Hello, World!');");

make_lang!(Perl);
impl Language for Perl {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["perl", "pl", "pm"]
    }
    fn container_options(&self) -> ContainerOptionsBuilder {
        let mut builder = ContainerOptions::builder("perl:slim");
        builder.cmd(vec!["perl", "code"]);
        builder
    }
}
test_lang!(Perl, "print 'Hello, World!\n'");

make_lang!(Python);
impl Language for Python {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["python", "py", "gyp"]
    }
    fn container_options(&self) -> ContainerOptionsBuilder {
        let mut builder = ContainerOptions::builder("python:alpine");
        // Make python run unbuffered. If you don't then we get weird orderings of messages
        builder
            .cmd(vec!["python", "code"])
            .env(["PYTHONUNBUFFERED=1"]);
        builder
    }
}
test_lang!(Python, "print('Hello, World!')");

make_lang!(Ruby);
impl Language for Ruby {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["ruby", "rb", "gemspec", "podspec", "thor", "irb"]
    }
    fn container_options(&self) -> ContainerOptionsBuilder {
        let mut builder = ContainerOptions::builder("ruby:alpine");
        builder.cmd(vec!["ruby", "code"]);
        builder
    }
}
test_lang!(Ruby, "puts 'Hello, World!'");

make_lang!(Rust);
impl Language for Rust {
    fn codes(&self) -> &[Ascii<&str>] {
        codes!["rust", "rs"]
    }
    fn container_options(&self) -> ContainerOptionsBuilder {
        let mut builder = ContainerOptions::builder("rust:alpine");
        builder.cmd(vec!["sh", "-c", "rustc code -o exe && ./exe"]);
        builder
    }
}
test_lang!(
    Rust,
    r#"
fn main() {
    println!("Hello, World!");
}"#
);
