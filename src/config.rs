use std::default::Default;
use std::iter::Iterator;
use std::path::PathBuf;


#[derive(Clone, Debug, PartialEq)]
pub struct TranspoConfig {
    pub max_upload_age_minutes: usize,
    pub max_upload_size_bytes: usize,
    pub max_storage_size_bytes: usize,
    pub port: usize,
    pub storage_dir: PathBuf,
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

            storage_dir: PathBuf::from("./transpo_storage"),

            port: 8123
        }
    }
}

impl<I, S> From<I> for TranspoConfig
where I: Iterator<Item = S>,
      S: AsRef<str>
{
    fn from(args: I) -> Self {
        let mut config = Self::default();
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            let field = match arg.as_ref() {
                "-a" => &mut config.max_upload_age_minutes,
                "-u" => &mut config.max_upload_size_bytes,
                "-s" => &mut config.max_storage_size_bytes,
                "-p" => &mut config.port,
                "-d" => {
                    config.storage_dir = PathBuf::from(arg.as_ref());
                    continue;
                },
                _ => continue
            };

            if let Some(parsed) = args.peek().and_then(|a| a.as_ref().parse().ok()) {
                *field = parsed;
            }
        }

        config
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

        let actual_config = TranspoConfig::from(args.into_iter());

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
