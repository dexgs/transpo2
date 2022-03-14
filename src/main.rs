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
mod quotas;
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
use quotas::*;

use std::env;
use std::fs;
use std::sync::Arc;
use std::net::IpAddr;
use trillium::{Conn, Headers, State};
use trillium_websockets::{WebSocketConn, WebSocketConfig, websocket};
use trillium_router::{Router, RouterConnExt};
use trillium_askama::AskamaConnExt;
use trillium_static::{files, crate_relative_path};


const X_REAL_IP: &'static str = "X-Real-IP";

const WS_CONFIG: WebSocketConfig = WebSocketConfig {
    max_send_queue: None,
    max_message_size: Some(FORM_READ_BUFFER_SIZE * 2),
    max_frame_size: Some(FORM_READ_BUFFER_SIZE * 2),
    accept_unmasked_frames: false
};

#[derive(Clone)]
struct TranspoState {
    config: Arc<TranspoConfig>,
    accessors: Accessors,
    quotas: Option<Quotas>
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
            config.max_upload_age_minutes,
            config.storage_dir.to_owned(),
            db_backend, config.db_url.to_owned());

        trillium_main(config, db_backend);
    } else {
        eprintln!("A database connection is required!");
        std::process::exit(1);
    }
}

fn get_quotas_data(quotas: Option<Quotas>, headers: &Headers) -> Option<(Quotas, IpAddr)> {
    quotas.and_then(|q| Some((q, addr_from_headers(headers)?)))
}

fn addr_from_headers(headers: &Headers) -> Option<IpAddr> {
    headers
        .get_str(X_REAL_IP)
        .and_then(|a| a.parse().ok())
}

fn trillium_main(config: Arc<TranspoConfig>, db_backend: db::DbBackend) {
    env_logger::init();

    let index = IndexTemplate::from(config.as_ref());
    let about = AboutTemplate::from(config.as_ref());
    let quotas = if config.quota_bytes == 0 {
        None
    } else {
        Some(Quotas::from(config.as_ref()))
    };
    let accessors = Accessors::new();

    if let Some(quotas) = quotas.clone() {
        spawn_quotas_thread(quotas);
    }

    let state = TranspoState {
        config: config.clone(),
        accessors: accessors.clone(),
        quotas: quotas.clone(),
    };

    trillium_smol::config()
        .with_host("0.0.0.0")
        .with_port(config.port as u16)
        .run(
            Router::new()
                .get("/", move |conn: Conn| {
                    let index = index.clone();
                    async move { 
                        conn
                            .render(index)
                            .with_header("Clear-Site-Data", "\"storage\"")
                            .halt()
                    }
                })
                .get("/about", move |conn: Conn| {
                    let about = about.clone();
                    async move { 
                        conn
                            .render(about)
                            .with_header("Clear-Site-Data", "\"storage\"")
                            .halt()
                    }
                })
                .get("/download_worker.js", files(crate_relative_path!("www/js")))
                .get("/js/*", files(crate_relative_path!("www/js")))
                .get("/css/*", files(crate_relative_path!("www/css")))
                .get("/res/*", files(crate_relative_path!("www/res")))
                .get("/templates/*", files(crate_relative_path!("templates")))
                .post("/upload", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();
                    let quotas_data = get_quotas_data(state.quotas, conn.headers());

                    async move {
                        upload::handle_post(conn, state.config, db_backend, quotas_data).await
                    }
                }))
                .get("/upload", (State::new(state.clone()), websocket(move |mut conn: WebSocketConn| {
                    let state = conn.take_state::<TranspoState>().unwrap();
                    let quotas_data = get_quotas_data(state.quotas, conn.headers());

                    async move {
                        drop(upload::handle_websocket(conn, state.config, db_backend, quotas_data).await)
                    }
                }).with_protocol_config(WS_CONFIG)))
                .get("/:file_id", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();
                    let file_id = conn.param("file_id").unwrap().to_owned();
                    let app_name = state.config.app_name.clone();

                    async move {
                        if file_id.len() == base64_encode_length(ID_LENGTH) {
                            let template = DownloadTemplate {
                                file_id,
                                app_name,
                                has_password: conn.querystring() != "nopass"
                            };

                            conn
                                .render(template)
                                .with_header("Clear-Site-Data", "\"storage\"")
                                .halt()
                        } else {
                            http_errors::error_404(conn, state.config)
                        }
                    }
                }))
                .get("/:file_id/dl", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();
                    let file_id = conn.param("file_id").unwrap().to_owned();

                    async move {
                        download::handle(conn, file_id, state.config, state.accessors, db_backend).await
                    }
                }))
        );
}
