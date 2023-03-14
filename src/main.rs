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
mod quotas;
mod http_errors;
mod translations;

#[macro_use]
extern crate diesel;

use config::*;
use translations::*;
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

const WS_UPLOAD_CONFIG: WebSocketConfig = WebSocketConfig {
    max_send_queue: None,
    max_message_size: Some(FORM_READ_BUFFER_SIZE * 2),
    max_frame_size: Some(FORM_READ_BUFFER_SIZE * 2),
    accept_unmasked_frames: false
};

const ID_STRING_LENGTH: usize = base64_encode_length(ID_LENGTH);


#[derive(Clone)]
struct TranspoState {
    config: Arc<TranspoConfig>,
    translations: Arc<Translations>,
    accessors: Accessors,
    quotas: Option<Quotas>
}

fn main() {
    let mut config = TranspoConfig::default();
    config.parse_vars(env::vars());
    config.parse_args(env::args());

    if !config.quiet {
        println!("Running with: {:#?}", &config);
    }

    let translations = translations::Translations::new(
            &config.translations_dir,
            &config.default_lang)
        .expect("Loading translations");

    fs::create_dir_all(&config.storage_dir)
        .expect("Creating storage directory");

    if let Some(db_backend) = db::parse_db_backend(&config.db_url) {
        let db_connection = db::establish_connection(db_backend, &config.db_url);
        db::run_migrations(&db_connection, &config.migrations_dir);

        let config = Arc::new(config);
        let translations = Arc::new(translations);

        spawn_cleanup_thread(
            config.read_timeout_milliseconds,
            config.storage_dir.to_owned(),
            db_backend, config.db_url.to_owned());

        trillium_main(config, translations, db_backend);
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

// query -> cookie -> default
fn get_lang(conn: &mut Conn, default_lang: &str) -> String {
    let mut query_lang = None;
    let query_string = conn.querystring().to_string();
    for arg in query_string.split("&") {
        if let Some((key, value)) = arg.split_once("=") {
            if key.trim() == "lang" {
                let value = value.trim();
                query_lang = Some(value);
                conn.headers_mut()
                    .insert("Set-Cookie", format!("lang={}", value));
                break;
            }
        }
    }

    let mut cookie_lang = None;
    if let Some(cookie) = conn.headers().get_str("Cookie") {
        for arg in cookie.split(";") {
            if let Some((key, value)) = arg.split_once("=") {
                if key.trim() == "lang" {
                    cookie_lang = Some(value.trim());
                    break;
                }
            }
        }
    }

    query_lang.or(cookie_lang).unwrap_or(default_lang).to_string()
}

fn trillium_main(config: Arc<TranspoConfig>, translations: Arc<Translations>, db_backend: db::DbBackend) {
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
        translations: translations.clone(),
        accessors: accessors.clone(),
        quotas: quotas.clone(),
    };

    trillium_smol::config()
        .with_host("0.0.0.0")
        .with_port(config.port as u16)
        .run(
            Router::new()
                .get("/", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();

                    async move {
                        let lang = get_lang(&mut conn, &state.config.default_lang);

                        let index = IndexTemplate::new(
                            state.config.as_ref(),
                            state.translations.names(),
                            &lang,
                            state.translations.get(&lang)
                            );

                        conn.render(index).halt()
                    }
                }))
                .get("/about", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();

                    async move {
                        let lang = get_lang(&mut conn, &state.config.default_lang);
                        let translation = state.translations.get(&lang);
                        let about = AboutTemplate::new(&state.config, translation);

                        conn.render(about).halt()
                    }
                }))
                .get("/download_worker.js", files(crate_relative_path!("www/js")))
                .get("/js/*", files(crate_relative_path!("www/js")))
                .get("/css/*", files(crate_relative_path!("www/css")))
                .get("/res/*", files(crate_relative_path!("www/res")))
                .post("/upload", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();
                    let quotas_data = get_quotas_data(state.quotas, conn.headers());

                    async move {
                        let lang = get_lang(&mut conn, &state.config.default_lang);
                        let translation = state.translations.get(&lang);

                        upload::handle_post(conn, state.config, translation, db_backend, quotas_data).await
                    }
                }))
                .get("/upload", (State::new(state.clone()), websocket(move |mut conn: WebSocketConn| {
                    let state = conn.take_state::<TranspoState>().unwrap();
                    let quotas_data = get_quotas_data(state.quotas, conn.headers());

                    async move {
                        drop(upload::handle_websocket(conn, state.config, db_backend, quotas_data).await)
                    }
                }).with_protocol_config(WS_UPLOAD_CONFIG)))
                .get("/:file_id", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();
                    let file_id = conn.param("file_id").unwrap().to_owned();
                    let app_name = state.config.app_name.clone();

                    async move {
                        let lang = get_lang(&mut conn, &state.config.default_lang);
                        let translation = state.translations.get(&lang);

                        if file_id.len() == ID_STRING_LENGTH {
                            let template = DownloadTemplate {
                                file_id,
                                app_name,
                                has_password: conn.querystring() != "nopass",
                                t: translation
                            };

                            conn.render(template).halt()
                        } else {
                            http_errors::error_404(conn, state.config, translation)
                        }
                    }
                }))
                .get("/:file_id/info", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();
                    let file_id = conn.param("file_id").unwrap().to_owned();

                    async move {
                        let lang = get_lang(&mut conn, &state.config.default_lang);
                        let translation = state.translations.get(&lang);

                        download::info(
                            conn, file_id, state.config,
                            state.accessors, translation, db_backend).await
                    }
                }))
                .get("/:file_id/dl", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();
                    let file_id = conn.param("file_id").unwrap().to_owned();

                    async move {
                        let lang = get_lang(&mut conn, &state.config.default_lang);
                        let translation = state.translations.get(&lang);

                        download::handle(
                            conn, file_id, state.config,
                            state.accessors, translation, db_backend).await
                    }
                }))
                .get("/clear-data", move |conn: Conn| {
                    async move {
                        conn
                            .with_status(200)
                            .with_header("Clear-Site-Data", "\"storage\"")
                            .with_body("Cleared site data (including service worker)")
                            .halt()
                    }
                })
                .get("*", (State::new(state.clone()), move |mut conn: Conn| {
                    let state = conn.take_state::<TranspoState>().unwrap();

                    async move {
                        let lang = get_lang(&mut conn, &state.config.default_lang);
                        let translation = state.translations.get(&lang);

                        http_errors::error_404(conn, state.config, translation)
                    }
                }))
        );
}
