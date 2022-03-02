use std::default::Default;
use std::iter::Iterator;
use std::path::PathBuf;


#[derive(Clone, Debug, PartialEq)]
pub struct TranspoConfig {
    pub max_upload_age_minutes: usize,
    pub max_upload_size_bytes: usize,
    pub max_storage_size_bytes: usize,
    pub port: usize,
    pub compression_level: usize,
    pub storage_dir: PathBuf,
    pub db_url: String,
    pub app_name: String,
}

impl Default for TranspoConfig {
    fn default() -> Self {
        TranspoConfig {
            // 1 Week
            max_upload_age_minutes: 7 * 24 * 60,
            // 5gB
            max_upload_size_bytes: 5 * 1000 * 1000 * 1000,
            // 100gB
            max_storage_size_bytes: 100 * 1000 * 1000 * 1000,

            port: 8123,

            compression_level: 0,

            storage_dir: PathBuf::from("./transpo_storage"),

            db_url: "./transpo_storage/db.sqlite".to_string(),

            app_name: "Transpo".to_string()
        }
    }
}

impl TranspoConfig {
    pub fn parse_vars<I, S1, S2>(&mut self, vars: I)
    where I: Iterator<Item = (S1, S2)>,
          S1: AsRef<str>,
          S2: AsRef<str>
    {
        for (key, val) in vars {
            let field = match key.as_ref() {
                "TRANSPO_MAX_UPLOAD_AGE_MINUTES" => &mut self.max_upload_age_minutes,
                "TRANSPO_MAX_UPLOAD_SIZE_BYTES" => &mut self.max_upload_size_bytes,
                "TRANSPO_MAX_STORAGE_SIZE_BYTES" => &mut self.max_storage_size_bytes,
                "TRANSPO_PORT" => &mut self.port,
                "TRANSPO_COMPRESSION_LEVEL" => &mut self.compression_level,
                "TRANSPO_STORAGE_DIRECTORY" => {
                    self.storage_dir = PathBuf::from(val.as_ref());
                    continue;
                },
                "TRANSPO_DATABASE_URL" => {
                    self.db_url = val.as_ref().to_string();
                    continue;
                },
                "TRANSPO_APP_NAME" => {
                    self.app_name = val.as_ref().to_string();
                    continue;
                },
                _ => continue
            };

            if let Ok(parsed) = val.as_ref().parse() {
                *field = parsed;
            }
        }
    }

    pub fn parse_args<I, S>(&mut self, args: I)
    where I: Iterator<Item = S>,
          S: AsRef<str>
    {
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            let field = match arg.as_ref() {
                "-a" => &mut self.max_upload_age_minutes,
                "-u" => &mut self.max_upload_size_bytes,
                "-s" => &mut self.max_storage_size_bytes,
                "-p" => &mut self.port,
                "-c" => &mut self.compression_level,
                "-d" => {
                    if let Some(next) = args.peek() {
                        self.storage_dir = PathBuf::from(next.as_ref());
                    }
                    continue;
                },
                "-D" => {
                    if let Some(next) = args.peek() {
                        self.db_url = next.as_ref().to_string();
                    }
                    continue;
                },
                "-n" => {
                    if let Some(next) = args.peek() {
                        self.app_name = next.as_ref().to_string();
                    }
                    continue;
                },
                _ => continue
            };

            if let Some(parsed) = args.peek().and_then(|a| a.as_ref().parse().ok()) {
                *field = parsed;
            }
        }
    }
}
