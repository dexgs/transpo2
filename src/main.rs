mod config;
mod templates;
mod multipart_form;
mod concurrency;
mod upload;
mod download;
mod random_bytes;
mod b64;
mod files;
mod constants;
mod db;
mod cleanup;
// mod rate_limit; not used for now...
mod http_errors;

#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;

use config::*;
use constants::*;
use b64::*;
use templates::*;
use concurrency::*;
use cleanup::*;

use std::env;
use std::fs;
use std::sync::Arc;
use trillium::{Conn, State};
use trillium_router::{Router, RouterConnExt};
use trillium_askama::AskamaConnExt;
use trillium_static::{files, crate_relative_path};

#[derive(Clone)]
struct TranspoState {
    config: Arc<TranspoConfig>,
    accessors: Accessors
}

fn main() {
    let mut config = TranspoConfig::default();
    config.parse_vars(env::vars());
    config.parse_args(env::args());
    println!("Running with: {:#?}", &config);

    fs::create_dir_all(&config.storage_dir)
        .expect("Creating storage directory");

    if let Some(db_backend) = db::parse_db_backend(&config.db_url) {
        let db_connection = db::establish_connection(db_backend, &config.db_url);
        db::run_migrations(&db_connection);

        let config = Arc::new(config);

        spawn_cleanup_thread(
            config.storage_dir.to_owned(),
            db_backend, config.db_url.to_owned());

        trillium_main(config, db_backend);
    } else {
        eprintln!("A database connection is required!");
        std::process::exit(1);
    }
}

fn trillium_main(config: Arc<TranspoConfig>, db_backend: db::DbBackend) {
    let index = IndexTemplate::from(config.as_ref());
    let about = AboutTemplate::from(config.as_ref());
    let accessors = Accessors::new();

    let state = TranspoState {
        config: config.clone(),
        accessors: accessors.clone()
    };

    trillium_smol::config()
        .with_host("0.0.0.0")
        .with_port(config.port as u16)
        .run(
            Router::new()
                .get("/", move |conn: Conn| {
                    let index = index.clone();
                    async move { conn.render(index).halt() }
                })
                .get("/about", move |conn: Conn| {
                    let about = about.clone();
                    async move { conn.render(about).halt() }
                })
                .get("/js/*", files(crate_relative_path!("www/js")))
                .get("/css/*", files(crate_relative_path!("www/css")))
                .get("/templates/*", files(crate_relative_path!("templates")))
                .post("/upload", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();

                    async move {
                        upload::handle(conn, state.config, db_backend).await
                    }
                }))
                .get("/:file_id", move |conn: Conn| {
                    let file_id = conn.param("file_id").unwrap().to_owned();
                    let app_name = config.app_name.clone();

                    async move {
                        if file_id.len() == base64_encode_length(ID_LENGTH) {
                            conn.render(DownloadTemplate { file_id: file_id, app_name: app_name })
                        } else {
                            http_errors::error_404(conn)
                        }
                    }
                })
                .get("/dl/:file_id", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();
                    let file_id = conn.param("file_id").unwrap().to_owned();

                    async move {
                        download::handle(conn, file_id, state.config, state.accessors, db_backend).await
                    }
                }))
        );
}
