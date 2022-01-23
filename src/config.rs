use std::default::Default;
use std::iter::Iterator;


#[derive(Debug)]
pub struct TranspoConfig {
    pub max_upload_age_minutes: usize,
    pub max_upload_size_bytes: usize,
    pub max_storage_size_bytes: usize,
    pub port: usize
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

            port: 8123
        }
    }
}

impl<I> From<I> for TranspoConfig
where I: Iterator<Item = String>
{
    fn from(args: I) -> Self {
        let mut config = Self::default();
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            let field = match arg.as_str() {
                "-a" => &mut config.max_upload_age_minutes,
                "-u" => &mut config.max_upload_size_bytes,
                "-s" => &mut config.max_storage_size_bytes,
                "-p" => &mut config.port,
                _ => continue
            };

            if let Some(parsed) = args.peek().and_then(|a| a.parse().ok()) {
                *field = parsed;
            }
        }

        config
    }
}
