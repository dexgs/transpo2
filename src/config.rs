use std::default::Default;
use std::iter::Iterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;


#[derive(Clone, Debug, PartialEq)]
pub struct TranspoConfig {
    pub max_upload_age_minutes: usize,
    pub max_upload_size_bytes: usize,
    pub max_storage_size_bytes: usize,
    pub port: usize,
    pub storage_dir: Arc<PathBuf>,
    pub db_url: Arc<String>
}

impl Default for TranspoConfig {
    fn default() -> Self {
        let storage_dir = PathBuf::from("./transpo_storage");

        TranspoConfig {
            // 1 Week
            max_upload_age_minutes: 7 * 24 * 60,
            // 5gB
            max_upload_size_bytes: 5 * 1000 * 1000 * 1000,
            // 100gB
            max_storage_size_bytes: 100 * 1000 * 1000 * 1000,

            port: 8123,

            storage_dir: Arc::new(PathBuf::from("./transpo_storage")),

            db_url: Arc::new("./transpo_storage/db.sqlite".to_string()),
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
                "TRANSPO_STORAGE_DIRECTORY" => {
                    self.storage_dir = Arc::new(PathBuf::from(val.as_ref()));
                    continue;
                },
                "TRANSPO_DATABASE_URL" => {
                    self.db_url = Arc::new(val.as_ref().to_string());
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
                "-d" => {
                    self.storage_dir = Arc::new(PathBuf::from(arg.as_ref()));
                    continue;
                },
                "-D" => {
                    self.db_url = Arc::new(arg.as_ref().to_string());
                    continue;
                }
                _ => continue
            };

            if let Some(parsed) = args.peek().and_then(|a| a.as_ref().parse().ok()) {
                *field = parsed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rand::Rng;
    use crate::TranspoConfig;

    fn parse_random<R>(mut rng: R, args: &mut Vec<String>)
    where R: Rng
    {
        args.clear();
        let mut expected_config = TranspoConfig::default();

        if rng.gen() {
            expected_config.max_upload_age_minutes = rng.gen();
            args.push("-a".to_owned());
            args.push(expected_config.max_upload_age_minutes.to_string());
        }
        if rng.gen() {
            expected_config.max_upload_size_bytes = rng.gen();
            args.push("-u".to_owned());
            args.push(expected_config.max_upload_size_bytes.to_string());
        }
        if rng.gen() {
            expected_config.max_storage_size_bytes = rng.gen();
            args.push("-s".to_owned());
            args.push(expected_config.max_storage_size_bytes.to_string());
        }
        if rng.gen() {
            expected_config.port = rng.gen();
            args.push("-p".to_owned());
            args.push(expected_config.port.to_string());
        }

        let mut actual_config = TranspoConfig::default();
        actual_config.parse_args(args.into_iter());

        assert_eq!(expected_config, actual_config);
    }

    #[test]
    fn test_parsing() {
        let mut rng = rand::thread_rng();
        let mut args: Vec<String> = Vec::with_capacity(8);
        for _ in 0..50 {
            parse_random(&mut rng, &mut args);
        }
    }
}
