use std::default::Default;
use std::iter::Iterator;
use std::path::PathBuf;


const HELP_MSG: &'static str = "
Transpo accepts configuration options, either as command line arguments or as
environment variables. The available options are as follows:

(This list is formatted as `argument/environment variable <value>: description`)

 -a / TRANSPO_MAX_UPLOAD_AGE_MINUTES     <number> : maximum time in minutes before uploads expire
 -u / TRANSPO_MAX_UPLOAD_SIZE_BYTES      <number> : maximum size allowed for a single upload
 -s / TRANSPO_MAX_STORAGE_SIZE_BYTES     <number> : maximum total size of all uploads currently stored
 -p / TRANSPO_PORT                       <number> : port to which Transpo will bind
 -c / TRANSPO_COMPRESSION_LEVEL      <number 0-9> : gzip compression level to use when creating zip archives
 -q / TRANSPO_QUOTA_BYTES                <number> : maximum number of bytes a single IP address can upload
                                                    within the quota interval. (set to 0 to disable)
 -i / TRANSPO_QUOTA_INTERVAL_MINUTES     <number> : number of minutes before quotas are reset
 -t / TRANSPO_READ_TIMEOUT_MILLISECONDS  <number> : number of milliseconds before which each read must
                                                    complete or else the upload is aborted
 -d / TRANSPO_STORAGE_DIRECTORY            <path> : path to the directory where Transpo will store uploads
 -D / TRANSPO_DATABASE_URL             <path/url> : URL to which database connections will be made
 -n / TRANSPO_APP_NAME                   <string> : name shown in web interface
 -h /                                             : print this help message and exit
";


#[derive(Clone, Debug, PartialEq)]
pub struct TranspoConfig {
    pub max_upload_age_minutes: usize,
    pub max_upload_size_bytes: usize,
    pub max_storage_size_bytes: usize,
    pub port: usize,
    pub compression_level: usize,
    pub quota_bytes: usize,
    pub quota_interval_minutes: usize,
    pub read_timeout_milliseconds: usize,
    pub storage_dir: PathBuf,
    pub db_url: String,
    pub app_name: String,
}

impl Default for TranspoConfig {
    fn default() -> Self {
        TranspoConfig {
            // 1 Week
            max_upload_age_minutes: 7 * 24 * 60,
            // 5GB
            max_upload_size_bytes: 5 * 1000 * 1000 * 1000,
            // 100GB
            max_storage_size_bytes: 100 * 1000 * 1000 * 1000,

            port: 8123,

            compression_level: 0,

            // 0B (disabled)
            quota_bytes: 0,
            // 1 Hour
            quota_interval_minutes: 60,

            read_timeout_milliseconds: 500,

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
                "TRANSPO_QUOTA_BYTES" => &mut self.quota_bytes,
                "TRANSPO_QUOTA_INTERVAL_MINUTES" => &mut self.quota_interval_minutes,
                "TRANSPO_READ_TIMEOUT_MILLISECONDS" => &mut self.read_timeout_milliseconds,
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
                "-q" => &mut self.quota_bytes,
                "-i" => &mut self.quota_interval_minutes,
                "-t" => &mut self.read_timeout_milliseconds,
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
                "-h" | "--help" => {
                    println!("{}", HELP_MSG);
                    std::process::exit(1);
                }
                _ => continue
            };

            if let Some(parsed) = args.peek().and_then(|a| a.as_ref().parse().ok()) {
                *field = parsed;
            }
        }
    }
}
