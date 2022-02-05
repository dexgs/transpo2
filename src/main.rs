mod config;
mod templates;
mod multipart_form;
mod concurrency;
mod upload;
mod random_bytes;
mod b64;
mod files;
mod constants;

use config::*;
use templates::*;

use std::env;
use std::fs;
use trillium::Conn;
use trillium_router::Router;
use trillium_askama::AskamaConnExt;
use trillium_static::{files, crate_relative_path};
use diesel::prelude::*;

fn main() {
    let mut config = TranspoConfig::default();
    config.parse_vars(env::vars());
    config.parse_args(env::args());
    println!("Running with: {:?}", &config);

    fs::create_dir_all(&*config.storage_dir)
        .expect("Creating storage directory");

    if config.db_url.starts_with("mysql://") {
        #[cfg(feature = "mysql")]
        return trillium_main::<MysqlConnection>(config);
    } else if config.db_url.starts_with("postgresql://") {
        #[cfg(feature = "postgres")]
        return trillium_main::<PgConnection>(config);
    } else {
        #[cfg(feature = "sqlite")]
        return trillium_main::<SqliteConnection>(config);
    }

    eprintln!("A database connection is required!");
    std::process::exit(1);
}

fn trillium_main<C>(config: TranspoConfig) 
where C: Connection
{
    let index = IndexTemplate::from(&config);
    let accessors = concurrency::Accessors::new();

    trillium_smol::config()
        .with_host("0.0.0.0")
        .with_port(config.port as u16)
        .run(
            Router::new()
                .get("/", move |conn: Conn| async move { conn.render(index) })
                .get("/js/*", files(crate_relative_path!("www/js")))
                .get("/css/*", files(crate_relative_path!("www/css")))
                .get("/:file_id", |conn: Conn| async { conn.ok("blah!") })
                .post("/upload", move |conn: Conn| {
                    let storage_dir = config.storage_dir.clone();
                    let db_url = config.db_url.clone();
                    let accessors = accessors.clone();
                    async move {
                        if let Ok(connection) = C::establish(&db_url) {
                            upload::handle(conn, config.max_upload_size_bytes, accessors, connection, storage_dir).await
                        } else {
                            conn.with_body("Internal Server Error")
                                .with_status(500)
                                .halt()
                        }
                    }
                })
                //.get("/dl/:file_id")
        );
}
