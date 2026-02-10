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
 -q / TRANSPO_QUOTA_BYTES_TOTAL          <number> : maximum number of bytes a single IP address can upload
                                                    within the quota interval. (set to 0 to disable)
 -b / TRANSPO_QUOTA_BYTES_PER_MINUTE     <number> : number of bytes to refund to each quota per minute
 -t / TRANSPO_READ_TIMEOUT_MILLISECONDS  <number> : number of milliseconds before which each read must
                                                    complete or else the upload is aborted
 -d / TRANSPO_STORAGE_DIRECTORY            <path> : path to the directory where Transpo will store uploads
 -D / TRANSPO_DATABASE_URL             <path/url> : URL to which database connections will be made
 -m / TRANSPO_MIGRATIONS_DIRECTORY         <path> : path to the directory containing migration directories.
 -l / TRANSPO_DEFAULT_LANGUAGE           <string> : language code of default language.
 -T / TRANSPO_TRANSLATIONS_DIRECTORY       <path> : path to the translations directory.
 --pool-max / TRANSPO_DB_POOL_MAX        <number> : maximum database pool connection count
 --pool-min / TRANSPO_DB_POOL_MIN        <number> : minimum database pool *idle* connection count
 -n / TRANSPO_APP_NAME                   <string> : name shown in web interface
 -Q /                                             : quiet: do not print configuration on start
 -h /                                             : print this help message and exit
";


#[derive(Clone, Debug, PartialEq)]
pub struct TranspoConfig {
    pub max_upload_age_minutes: usize,
    pub max_upload_size_bytes: usize,
    pub max_storage_size_bytes: usize,
    pub port: usize,
    pub quota_bytes_total: usize,
    pub quota_bytes_per_minute: usize,
    pub read_timeout_milliseconds: usize,
    pub storage_dir: PathBuf,
    pub db_url: String,
    pub migrations_dir: PathBuf,
    pub default_lang: String,
    pub translations_dir: PathBuf,
    pub app_name: String,
    pub quiet: bool,

    pub max_db_pool_size: u32,
    pub min_db_pool_idle: Option<u32>
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

            // 0B (disabled)
            quota_bytes_total: 0,

            // 10GiB / hour
            quota_bytes_per_minute: 17895697,

            read_timeout_milliseconds: 800,

            storage_dir: PathBuf::from("./transpo_storage"),

            db_url: "./transpo_storage/db.sqlite".to_string(),

            migrations_dir: PathBuf::from("./"),

            default_lang: "en".to_string(),

            translations_dir: PathBuf::from("./translations"),

            app_name: "Transpo".to_string(),

            quiet: false,

            max_db_pool_size: 10,
            min_db_pool_idle: None
        }
    }
}

impl TranspoConfig {
    // parse config from environment variables
    pub fn parse_vars<I, S1, S2>(&mut self, vars: I)
    where I: Iterator<Item = (S1, S2)>,
          S1: AsRef<str>,
          S2: AsRef<str>
    {
        self.parse_options(vars);
    }

    // parse config from command line arguments
    pub fn parse_args<I, S>(&mut self, args: I)
    where I: Iterator<Item = S>,
          S: AsRef<str>
    {
        let mut args = args.peekable();
        let mut options = Vec::new();

        while let Some(arg) = args.next() {
            if arg.as_ref().starts_with('-') {
                let key = arg.as_ref().to_string();
                let value = args.peek()
                    .map(|s| s.as_ref())
                    .unwrap_or("").to_string();

                options.push((key, value));
            }
        }

        self.parse_options(options.into_iter());
    }

    fn parse_options<I, S1, S2>(&mut self, options: I)
    where I: Iterator<Item = (S1, S2)>,
          S1: AsRef<str>,
          S2: AsRef<str>
    {
        for (key, value) in options {
            let key = key.as_ref();
            let value = value.as_ref();

            match key {
                "-a" | "TRANSPO_MAX_UPLOAD_AGE_MINUTES" => {
                    self.max_upload_age_minutes = value.parse()
                        .expect("Parsing configured max upload age");
                },
                "-u" | "TRANSPO_MAX_UPLOAD_SIZE_BYTES" => {
                    self.max_upload_size_bytes = value.parse()
                        .expect("Parsing configured max upload file size");
                },
                "-s" | "TRANSPO_MAX_STORAGE_SIZE_BYTES" => {
                    self.max_storage_size_bytes = value.parse()
                        .expect("Parsing configured max total storage size");
                },
                "-p" | "TRANSPO_PORT" => {
                    self.port = value.parse()
                        .expect("Parsing configured port");
                },
                "-q" | "TRANSPO_QUOTA_BYTES_TOTAL" => {
                    self.quota_bytes_total = value.parse()
                        .expect("Parsing configured upload quota limit");
                },
                "-b" | "TRANSPO_QUOTA_BYTES_PER_MINUTE" => {
                    self.quota_bytes_per_minute = value.parse()
                        .expect("Parsing configured quota clear interval");
                },
                "-t" | "TRANSPO_READ_TIMEOUT_MILLISECONDS" => {
                    self.read_timeout_milliseconds = value.parse()
                        .expect("Parsing configured read timeout");
                },
                "-d" | "TRANSPO_STORAGE_DIRECTORY" => {
                    self.storage_dir = value.parse()
                        .expect("Parsing configured storage directory");
                },
                "-D" | "TRANSPO_DATABASE_URL" => {
                    self.db_url = value.parse()
                        .expect("Parsing configured storage directory");
                },
                "-m" | "TRANSPO_MIGRATIONS_DIRECTORY" => {
                    self.migrations_dir = value.parse()
                        .expect("Parsing configured migrations directory");
                },
                "-l" | "TRANSPO_DEFAULT_LANGUAGE" => {
                    self.default_lang = value.to_string();
                },
                "-T" | "TRANSPO_TRANSLATIONS_DIRECTORY" => {
                    self.translations_dir = value.parse()
                        .expect("Parsing configured translations directory");
                },
                "--pool-max" | "TRANSPO_DB_POOL_MAX" => {
                    self.max_db_pool_size = value.parse()
                        .expect("Parsing configured maximum database pool connection count");
                },
                "--pool-min" | "TRANSPO_DB_POOL_MIN" => {
                    self.min_db_pool_idle = Some(
                        value.parse()
                        .expect("Parsing configured minimum database pool idle connection count"));
                },
                "-n" | "TRANSPO_APP_NAME" => {
                    self.app_name = value.to_string();
                },
                "-h" | "--help" => {
                    println!("{}", HELP_MSG);
                    std::process::exit(1);
                },
                "-Q" => {
                    self.quiet = true;
                },
                _ => {}
            }
        }
    }
}
