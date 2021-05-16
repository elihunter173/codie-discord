use std::collections::HashMap;

use anyhow::anyhow;
use nom::{
    branch::alt,
    bytes::complete::{escaped_transform, is_not, tag},
    character::complete::{alpha1, alphanumeric1, char, multispace0, multispace1},
    combinator::{cut, map, recognize, value, verify},
    error::context,
    multi::{many0, separated_list0},
    sequence::{pair, preceded, separated_pair, terminated},
    IResult,
};

pub type Options<'s> = HashMap<&'s str, String>;

fn quoteless_string(i: &str) -> IResult<&str, &str> {
    verify(is_not(" \""), |s: &str| !s.is_empty())(i)
}

fn quoted_string(i: &str) -> IResult<&str, String> {
    preceded(
        char('"'),
        cut(terminated(
            escaped_transform(
                is_not("\\\""),
                '\\',
                alt((value("\\", tag("\\")), value("\"", tag("\"")))),
            ),
            char('"'),
        )),
    )(i)
}

fn string(i: &str) -> IResult<&str, String> {
    context(
        "string",
        alt((map(quoteless_string, String::from), quoted_string)),
    )(i)
}

fn identifier(i: &str) -> IResult<&str, &str> {
    context(
        "identifier",
        recognize(pair(
            alt((alpha1, tag("_"))),
            many0(alt((alphanumeric1, tag("_")))),
        )),
    )(i)
}

fn key_value(i: &str) -> IResult<&str, (&str, String)> {
    separated_pair(identifier, char('='), string)(i)
}

fn config(i: &str) -> IResult<&str, Vec<(&str, String)>> {
    terminated(separated_list0(multispace1, key_value), multispace0)(i)
}

// TODO: Make this generic with what it returns
pub fn parse_options(conf: &str) -> anyhow::Result<Options> {
    match config(conf) {
        Ok(("", vec)) => {
            let mut map = HashMap::new();
            for (key, val) in vec {
                if map.insert(key, val).is_some() {
                    return Err(anyhow!("duplicate key {:?}", key));
                }
            }
            Ok(map)
        }
        Ok((input, _)) => Err(anyhow!("did not consume entire input: {:?}", input)),
        Err(err) => Err(anyhow!("{}", err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! hashmap {
        { $($key:expr => $value:expr),* $(,)? } => ({
            let mut m = ::std::collections::HashMap::new();
            $(m.insert($key, $value);)*
            m
        })
    }

    #[test]
    fn test_simple() {
        assert_eq!(
            parse_options("KEY=VAL").unwrap(),
            hashmap!["KEY" => "VAL".into()]
        );
    }

    #[test]
    fn test_multiple() {
        assert_eq!(
            parse_options("K0=V0 K1=V1").unwrap(),
            hashmap!["K0" => "V0".into(), "K1" => "V1".into()]
        );
    }

    #[test]
    fn test_dup_keys() {
        match parse_options("K0=V0 K0=V1") {
            Ok(v) => panic!(v),
            Err(_) => (),
        }
    }

    #[test]
    fn test_quoted_string() {
        assert_eq!(
            quoted_string(r#""Hello, World!""#).unwrap(),
            ("", "Hello, World!".into()),
        );
    }

    #[test]
    fn test_quoted_string_escape() {
        assert_eq!(
            quoted_string(r#""He said \"Hi!\"""#).unwrap(),
            ("", r#"He said "Hi!""#.into()),
        );
    }

    // TODO: Should I make these tests language specific?
    #[test]
    fn test_python_opts() {
        assert_eq!(
            parse_options("version=3.8").unwrap(),
            hashmap!["version" => "3.8".into()],
        );
    }

    #[test]
    fn test_c_opts() {
        assert_eq!(
            parse_options(r#"CFLAGS="-O2 -march=native""#).unwrap(),
            hashmap!["CFLAGS" => "-O2 -march=native".into()],
        );
    }
}
