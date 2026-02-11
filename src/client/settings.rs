use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::fmt::{Debug, Display};
use std::env;
use std::io::Error;

use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};

use dirs::config_dir;

#[cfg(feature = "magic")]
use magic;


fn print_help(cmd: String) {
eprintln!("Transpo can be used to either upload or download files.

\x1b[0;1mUPLOAD USAGE:\x1b[m
{0} <Transpo server URL> <file path> [OPTIONS...]

UPLOAD OPTIONS:
--file:     set file path
--mime:     set mime type
--url:      set webpage URL
--ws:       set WebSocket URL
--days:     set expiry time limit in days
--hours:    set expiry time limit in hours
--minutes:  set expiry time limit in minutes
--limit:    set download limit
--password: set download password

EXAMPLE UPLOAD USAGE:
{0} https://example.com/transpo ~/Pictures/pic.jpg --hours 1 --minutes 30

\x1b[0;1mDOWNLOAD USAGE:\x1b[m
{0} <Transpo download URL> [OPTIONS...]

DOWNLOAD OPTIONS:
--file:     set file path
--replace:  overwrite the destination file if it already exists
--url:      set download URL
--key:      set decryption key
--password: set download password

EXAMPLE DOWNLOAD USAGE:
{0} https://example.com/transpo/<upload ID>#<decryption key>

\x1b[0;1mNOTES:\x1b[m
To prevent Transpo from interpreting a value as a flag, e.g. to write to a
file literally named \"--file\", include -- before it.

For example:
\"--file -- --file\" sets the file path to \"--file\".
\"--file --file\" results in an error.
", cmd);
}


trait Values {
    fn next_value(&mut self) -> Option<String> {
        None
    }
}

impl<V> Values for &mut V
where V: Values
{
    fn next_value(&mut self) -> Option<String> {
        (*self).next_value()
    }
}

impl Values for Option<String> {
    fn next_value(&mut self) -> Option<String> {
        self.take()
    }
}

struct SettingsFile {
    reader: Lines<BufReader<File>>
}

impl SettingsFile {
    async fn new<P>(path: P, section: &str) -> Result<Self, Error>
    where P: AsRef<Path>
    {
        let file = OpenOptions::new()
            .read(true)
            .open(path)
            .await?;
        let mut reader = BufReader::new(file).lines();

        // Iterate until we get to the specificed section
        while let Some(line) = reader.next_line().await? {
            if let Some(s) = Self::parse_section_header(&line) {
                if s == section {
                    break;
                }
            }
        }

        Ok(Self { reader })
    }

    // Get the section heading from a line,
    // e.g. get "upload" from line containing "[upload]"
    fn parse_section_header(line: &str) -> Option<&'_ str> {
        let line = line.trim();
        if !(line.starts_with("[") && line.ends_with("]")) {
            return None;
        }
        Some(&line[1..line.len() - 1])
    }

    // Returns next key-value pair (or None, if end of section is reached)
    async fn next_line(&mut self) -> Result<Option<(String, Option<String>)>, Error> {
        let line = loop {
            match self.reader.next_line().await? {
                Some(line) => match line.trim_start() {
                    // skip blank lines
                    "" => continue,
                    // skip comments
                    l if l.starts_with("#") => continue,
                    _ => break line
                },
                None => return Ok(None)
            };
        };

        // Return early if we reached a new section
        if Self::parse_section_header(&line).is_some() {
            return Ok(None);
        }

        // settings are given as "key = value"
        // keys without values are allowed
        let key_val = match line.split_once("=") {
            Some((key, value)) => (key.trim().to_string(), Some(value.trim().to_string())),
            None => (line.trim().to_string(), None)
        };

        Ok(Some(key_val))
    }
}


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
            f if f.starts_with("-") => Arg::Flag(self.inner.next()?),
            _ => Arg::Value(self.inner.next()?)
        })
    }
}

impl<I> Values for Args<I>
where I: Iterator<Item = String>
{
    // Get the next arg if it is a VALUE, not a flag like "--file"
    fn next_value(&mut self) -> Option<String> {
        let next = self.inner.peek()?;
        match next.as_ref() {
            // skip over "--" to allow passing values with literal 
            "--" => {
                self.inner.next()?;
                self.inner.next()
            },
            f if f.starts_with("--") => None,
            _ => self.inner.next()
        }
    }
}


enum UploadSetting {
    FilePath,
    MimeType,
    WebpageURL,
    WebSocketURL,
    Days,
    Hours,
    Minutes,
    DownloadLimit,
    Password
}

impl UploadSetting {
    fn from_flag(flag: &str) -> Option<Self> {
        Some(match flag {
            "--file" => Self::FilePath,
            "--mime" => Self::MimeType,
            "--url" => Self::WebpageURL,
            "--ws" => Self::WebSocketURL,
            "--days" => Self::Days,
            "--hours" => Self::Hours,
            "--minutes" => Self::Minutes,
            "--limit" => Self::DownloadLimit,
            "--password" => Self::Password,
            _ => return None
        })
    }

    fn from_config(field: &str) -> Option<Self> {
        Some(match field {
            "url" => Self::WebpageURL,
            "minutes" => Self::Minutes,
            _ => return None
        })
    }
}

/*
enum DownloadSetting {
    FilePath,
    Replace,
    WebpageURL,
    DecryptionKey
}
*/

fn parse_next_value<O, V>(option: &mut Option<O>, flag: &str, mut values: V, errors: &mut Vec<String>)
where O: FromStr<Err: Display> + Debug,
      V: Values
{
    let value = match values.next_value() {
        Some(v) => parse_value(&v, flag, errors),
        None => {
            errors.push(format!("{} requires a value", flag));
            None
        }
    };
    set_option(option, value);
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

fn set_option<V>(option: &mut Option<V>, value: Option<V>)
    -> bool
where V: Debug
{
    match value {
        Some(value) => {
            option.replace(value);
            true
        },
        None => false
    }
}

fn print_errors<I>(errors: I)
where I: IntoIterator<Item: Display>
{
    for error in errors.into_iter() {
        eprintln!("{}", error);
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

impl UploadArgs {
    async fn get() -> Result<Self, Vec<String>> {
        let mut this = Self::default();
        this.parse().await?;

        Ok(this)
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

    async fn parse(&mut self) -> Result<(), Vec<String>> {
        let mut errors = vec![];

        // Parse CLI args
        let mut env_args = env::args();
        env_args.next();
        let mut args = Args::new(env_args);

        while let Some(arg) = args.next_arg() {
            match arg {
                Arg::Flag(flag) => match UploadSetting::from_flag(&flag) {
                    Some(setting) => self.parse_setting(setting, flag, &mut args, &mut errors),
                    None => errors.push(format!("Invalid flag: {}", flag))
                },
                Arg::Value(value) => self.parse_value(value)
            }
        }

        // Parse config file
        let config_path = config_dir()
            .map(|path| path.join("transpo-client.conf"))
            .and_then(|path| if path.exists() {
                    Some(path)
                } else {
                    None
                });

        if let Some(config_path) = config_path {
            match SettingsFile::new(config_path, "upload").await {
                Ok(mut file) => loop {
                    match file.next_line().await {
                        Ok(Some((key, mut value))) => {
                            match UploadSetting::from_config(&key) {
                                Some(setting) => self.parse_setting(setting, key, &mut value, &mut errors),
                                None => errors.push(format!("Invalid config key: {}", key))
                            }
                        },
                        // End of config
                        Ok(_) => break,
                        // Read error
                        Err(e) => {
                            errors.push(format!("Error reading config file: {:?}", e));
                            break;
                        }
                    }
                },
                Err(e) => errors.push(format!("Opening config file failed: {:?}", e))
            }
        }

        // Make sure everything is set

        if self.url.is_none() {
            errors.push(String::from("Missing webpage URL"));
        }

        // If WebSocket URL is unset, try getting it from the Webpage URL
        if self.ws_url.is_none() {
            self.ws_url = self.url.as_ref().and_then(|url| http_to_ws(url));
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
            errors.push(String::from("Missing time limit"));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn parse_setting<V>(&mut self, setting: UploadSetting, name: String, values: V, errors: &mut Vec<String>)
    where V: Values
    {
        match setting {
            UploadSetting::FilePath => parse_next_value(&mut self.file_path, &name, values, errors),
            UploadSetting::MimeType => parse_next_value(&mut self.mime_type, &name, values, errors),
            UploadSetting::WebpageURL => parse_next_value(&mut self.url, &name, values, errors),
            UploadSetting::WebSocketURL => parse_next_value(&mut self.ws_url, &name, values, errors),
            UploadSetting::Days => parse_next_value(&mut self.days, &name, values, errors),
            UploadSetting::Hours => parse_next_value(&mut self.hours, &name, values, errors),
            UploadSetting::Minutes => parse_next_value(&mut self.minutes, &name, values, errors),
            UploadSetting::DownloadLimit => parse_next_value(&mut self.download_limit, &name, values, errors),
            UploadSetting::Password => parse_next_value(&mut self.password, &name, values, errors)
        }
    }

    fn parse_value(&mut self, value: String) {
        match http_to_ws(&value) {
            Some(ws_url) => {
                set_option(&mut self.url, Some(value));
                set_option(&mut self.ws_url, Some(ws_url));
            }
            None => { set_option(&mut self.file_path, Some(PathBuf::from(value))); }
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

// Try getting the WebSocket endpoint from the WebPage URL
fn http_to_ws(value: &str) -> Option<String> {
    match value.split_once("://")? {
        ("http", path) => Some(format!("ws://{}/upload", path)),
        ("https", path) => Some(format!("wss://{}/upload", path)),
        _ => None
    }
}


#[derive(Default)]
struct DownloadArgs {
    url: Option<String>,
    key: Option<String>,
    password: Option<String>,
    file_path: Option<PathBuf>,
    force: Option<bool>,
    is_password_protected: bool
}

impl DownloadArgs {
    fn get() -> Result<Self, Vec<String>> {
        let mut env_args = env::args();
        env_args.next();
        let args = Args::new(env_args);
        let mut this = Self::default();
        this.parse(args)?;
        Ok(this)
    }

    fn settings(self) -> DownloadSettings {
        DownloadSettings {
            url: self.url.unwrap(),
            key: self.key.unwrap(),
            password: self.password,
            file_path: self.file_path,
            force: self.force.unwrap_or(false)
        }
    }

    fn parse<I>(&mut self, mut args: Args<I>) -> Result<(), Vec<String>>
    where I: Iterator<Item = String>
    {
        let mut errors = vec![];
        while let Some(arg) = args.next_arg() {
            match arg {
                Arg::Flag(flag) => self.parse_flag(flag, &mut args, &mut errors),
                Arg::Value(value) => self.parse_value(value)
            }
        }

        if self.url.is_some() {
            if self.is_password_protected && self.password.is_none() {
                errors.push(String::from("URL is password protected, but no password was specified"))
            }
        } else {
            errors.push(String::from("Missing download URL"));
        }

        if self.key.is_none() {
            errors.push(String::from("Missing decryption key"));
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
            "--url" => parse_next_value(&mut self.url, &flag, args, errors),
            "--key" => parse_next_value(&mut self.key, &flag, args, errors),
            "--password" => parse_next_value(&mut self.password, &flag, args, errors),
            "--file" => parse_next_value(&mut self.file_path, &flag, args, errors),
            "--replace" => { set_option(&mut self.force, Some(true)); },
            _ => errors.push(format!("Invalid option: {}", flag))
        }
    }

    fn parse_value(&mut self, value: String) {
        match split_url(&value) {
            Some((url, is_password_protected, key)) => {
                if set_option(&mut self.url, Some(url)) {
                    self.is_password_protected = is_password_protected;
                }
                set_option(&mut self.key, Some(key));
            },
            None => { set_option(&mut self.file_path, Some(PathBuf::from(value))); }
        }
    }
}

fn split_url(value: &str) -> Option<(String, bool, String)> {
    if !value.starts_with("http://") && !value.starts_with("https://") {
        return None;
    }

    let (url, key) = value.split_once("#")?;

    let (url, is_password_protected) = match url.split_once("?") {
        Some((url, query)) => {
            let mut is_password_protected = true;
            for arg in query.split("&") {
                if arg == "nopass" {
                    is_password_protected = false;
                    break;
                }
            }
            (url, is_password_protected)
        },
        None => (url, true)
    };

    let url = if !url.ends_with("/dl") {
        format!("{}/dl", url)
    } else {
        url.to_owned()
    };

    let key = key.to_owned();

    Some((url, is_password_protected, key))
}


pub struct DownloadSettings {
    pub url: String,
    pub key: String,
    pub password: Option<String>,
    pub file_path: Option<PathBuf>,
    pub force: bool
}


pub enum Settings {
    Upload(UploadSettings),
    Download(DownloadSettings)
}

impl Settings {
    pub async fn from_args() -> Self {
        let cmd = env::args().nth(0).unwrap();

        if let Some("--help") = env::args().nth(1).as_deref() {
            print_help(cmd);
            std::process::exit(1);
        }

        let upload_args = UploadArgs::get().await;
        let download_args = DownloadArgs::get();

        if let Ok(download_args) = download_args {
            Self::Download(download_args.settings())
        } else if let Ok(upload_args) = upload_args {
            Self::Upload(upload_args.settings())
        } else {
            debug_assert!(download_args.is_err() && upload_args.is_err());

            eprintln!("\x1b[0;1mInvalid upload settings:\x1b[m");
            print_errors(upload_args.err().unwrap());

            eprintln!("\n\x1b[0;1mInvalid download settings:\x1b[m");
            print_errors(download_args.err().unwrap());

            eprintln!("\nFor usage instructions, run {} --help", cmd);

            std::process::exit(1);
        }
    }
}
