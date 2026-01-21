use std::iter::Peekable;
use std::path::PathBuf;
use std::str::FromStr;
use std::fmt::{Debug, Display};
use std::env;

#[cfg(feature = "magic")]
use magic;


enum Arg {
    Flag(String),
    Value(String)
}

struct Args<I>
where I: Iterator<Item = String>
{
    inner: Peekable<I>
}

impl<I> Args<I>
where I: Iterator<Item = String>
{
    fn new(inner: I) -> Self {
        Self {
            inner: inner.peekable()
        }
    }

    fn next_arg(&mut self) -> Option<Arg> {
        let next = self.inner.peek()?;
        Some(match next.as_ref() {
            "--" => {
                self.inner.next()?;
                Arg::Value(self.inner.next()?)
            },
            f if f.starts_with("--") => Arg::Flag(self.inner.next()?),
            _ => Arg::Value(self.inner.next()?)
        })
    }

    // Get the next arg if it is a VALUE, not a flag like "--file"
    fn next_value(&mut self) -> Option<String> {
        let next = self.inner.peek()?;
        match next.as_ref() {
            "--" => {
                self.inner.next()?;
                self.inner.next()
            },
            f if f.starts_with("--") => None,
            _ => self.inner.next()
        }
    }
}


#[derive(Default)]
struct UploadArgs {
    file_path: Option<PathBuf>,
    mime_type: Option<String>,
    url: Option<String>,
    ws_url: Option<String>,
    days: Option<usize>,
    hours: Option<usize>,
    minutes: Option<usize>,
    password: Option<String>,
    download_limit: Option<usize>
}

#[derive(Debug)]
pub struct UploadSettings {
    pub file_path: PathBuf,
    pub mime_type: String,
    pub url: String,
    pub ws_url: String,
    pub minutes: usize,
    pub password: Option<String>,
    pub download_limit: Option<usize>
}

impl UploadSettings {
    pub fn from_args() -> Self {
        UploadArgs::get().settings()
    }
}

impl UploadArgs {
    fn get() -> Self {
        let mut env_args = env::args();
        env_args.next();
        let args = Args::new(env_args);
        let mut this = Self::default();
        match this.parse(args) {
            Ok(_) => this,
            Err(errors) => {
                for err in errors.into_iter() {
                    eprintln!("{}", err);
                }
                std::process::exit(1);
            }
        }
    }

    fn settings(self) -> UploadSettings {
        let minutes = self.minutes();
        UploadSettings {
            file_path: self.file_path.unwrap(),
            mime_type: self.mime_type.unwrap(),
            url: self.url.unwrap(),
            ws_url: self.ws_url.unwrap(),
            minutes: minutes,
            password: self.password,
            download_limit: self.download_limit
        }
    }

    fn minutes(&self) -> usize {
        self.minutes.unwrap_or(0)
            + 60 * self.hours.unwrap_or(0)
            + 24 * 60 * self.days.unwrap_or(0)
    }

    fn parse<I>(&mut self, mut args: Args<I>) -> Result<(), Vec<String>>
    where I: Iterator<Item = String>
    {
        let mut errors = vec![];
        while let Some(arg) = args.next_arg() {
            match arg {
                Arg::Flag(flag) => self.parse_flag(flag, &mut args, &mut errors),
                Arg::Value(value) => self.parse_value(value, &mut errors)
            }
        }

        if self.url.is_none() {
            errors.push(String::from("Missing webpage URL"));
        }

        if self.ws_url.is_none() {
            errors.push(String::from("Missing WebSocket URL"));
        }

        match self.file_path.as_ref() {
            Some(file_path) => if self.mime_type.is_none() {
                self.mime_type = Some(match get_mime_type(file_path) {
                    Some(mime_type) => mime_type,
                    None => String::from("application/octet-stream")
                });
            },
            None => errors.push(String::from("Missing file to upload"))
        }

        if self.minutes() == 0 {
            errors.push(String::from("Time limit must be longer than 0 minutes"));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn parse_flag<I>(&mut self, flag: String, args: &mut Args<I>, errors: &mut Vec<String>)
    where I: Iterator<Item = String>
    {
        match flag.as_ref() {
            "--file" => set_option_next_value(&mut self.file_path, &flag, args, errors),
            "--mime" => set_option_next_value(&mut self.mime_type, &flag, args, errors),
            "--url" => set_option_next_value(&mut self.url, &flag, args, errors),
            "--ws" => set_option_next_value(&mut self.ws_url, &flag, args, errors),
            "--days" => set_option_next_value(&mut self.days, &flag, args, errors),
            "--hours" => set_option_next_value(&mut self.hours, &flag, args, errors),
            "--minutes" => set_option_next_value(&mut self.minutes, &flag, args, errors),
            "--limit" => set_option_next_value(&mut self.download_limit, &flag, args, errors),
            "--password" => set_option_next_value(&mut self.password, &flag, args, errors),
            _ => errors.push(format!("Invalid option: `{}`", flag))
        }
    }

    fn parse_value(&mut self, value: String, errors: &mut Vec<String>) {
        match http_to_ws(&value) {
            Some(ws_url) => {
                set_option(&mut self.url, Some(value), "Web URL", errors);
                set_option(&mut self.ws_url, Some(ws_url), "WebSocket URL", errors);
            }
            None => set_option(&mut self.file_path, Some(PathBuf::from(value)), "file", errors)
        }
    }
}

#[cfg(feature = "magic")]
fn get_mime_type(path: &PathBuf) -> Option<String> {
    magic::Cookie::open(magic::cookie::Flags::MIME_TYPE).ok()?
        .load(&magic::cookie::DatabasePaths::default()).ok()?
        .file(path).ok()
}
#[cfg(not(feature = "magic"))]
fn get_mime_type(_path: &PathBuf) -> Option<String> {
    None
}

fn http_to_ws(value: &str) -> Option<String> {
    match value.split_once("://")? {
        ("http", path) => Some(format!("ws://{}/upload", path)),
        ("https", path) => Some(format!("wss://{}/upload", path)),
        _ => None
    }
}

fn set_option_next_value<I, V>(option: &mut Option<V>, flag: &str, args: &mut Args<I>, errors: &mut Vec<String>)
    where I: Iterator<Item = String>,
          V: FromStr<Err: Display> + Debug
{
    let value = match args.next_value() {
        Some(v) => parse_value(&v, flag, errors),
        None => {
            errors.push(format!("{} requires a value", flag));
            None
        }
    };
    set_option(option, value, flag, errors);
}

fn parse_value<V>(value: &str, flag: &str, errors: &mut Vec<String>) -> Option<V>
where V: FromStr<Err: Display> + Debug
{
    match V::from_str(value) {
        Ok(v) => Some(v),
        Err(err) => {
            errors.push(format!("`{}`: Error parsing `{}`: {}", flag, value, err));
            None
        }
    }
}

fn set_option<V>(option: &mut Option<V>, value: Option<V>, flag: &str, errors: &mut Vec<String>)
    where V: Debug
{
    match option {
        None => if let Some(value) = value
            { option.replace(value);
        },
        Some(v) => match value {
            Some(value) => errors.push(format!("{} cannot be re-assigned to {:?}, it was previously set to {:?}", flag, value, v)),
            None => errors.push(format!("{} cannot be re-assigned, it was previously set to {:?}", flag, v))
        }
    }
}
