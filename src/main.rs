extern crate diesel;

use transpo2::config::*;
use transpo2::translations::*;
use transpo2::constants::*;
use transpo2::b64::*;
use transpo2::templates::*;
use transpo2::concurrency::*;
use transpo2::cleanup::*;
use transpo2::quotas::*;
use transpo2::storage_limit::*;
use transpo2::http_errors;
use transpo2::download;
use transpo2::upload;
use transpo2::db;
use transpo2::translations;

use std::env;
use std::fs;
use std::sync::Arc;
use std::net::IpAddr;
use trillium_tokio::tokio::runtime::Builder;
use trillium::{Conn, Headers, state};
use trillium_websockets::{WebSocketConn, WebSocketConfig, websocket};
use trillium_router::{Router, RouterConnExt};
use trillium_askama::AskamaConnExt;
use trillium_static::{files, crate_relative_path};


const X_REAL_IP: &'static str = "X-Real-IP";

const WS_UPLOAD_CONFIG: WebSocketConfig = WebSocketConfig {
    #[allow(deprecated)]
    max_send_queue: None, // This field no longer does anything
    write_buffer_size: 128,
    max_write_buffer_size: 256,
    max_message_size: Some(FORM_READ_BUFFER_SIZE + 64),
    max_frame_size: Some(FORM_READ_BUFFER_SIZE + 64),
    accept_unmasked_frames: false
};

const ID_STRING_LENGTH: usize = base64_encode_length(ID_LENGTH);


#[derive(Clone)]
struct TranspoState {
    config: Arc<TranspoConfig>,
    translations: Arc<Translations>,
    accessors: Accessors,
    quotas: Option<Quotas>,
    storage_limit: StorageLimit,
    db_pool: Arc<db::DbConnectionPool>
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

    let db_pool = Arc::new(db::DbConnectionPool::from(&config));
    let mut c = db_pool.get().expect("Getting database connection to run migrations");
    db::run_migrations(&mut c, &config.migrations_dir);
    let config = Arc::new(config);
    let translations = Arc::new(translations);

    trillium_main(config, translations, db_pool);
}

fn get_quota(quotas: Option<Quotas>, headers: &Headers) -> Quota {
    let addr = addr_from_headers(headers);
    match addr.zip(quotas) {
        Some((addr, quotas)) => quotas.get(addr),
        None => Quota::unlimited()
    }
}

fn addr_from_headers(headers: &Headers) -> Option<IpAddr> {
    headers
        .get_str(X_REAL_IP)
        .and_then(|a| a.parse().ok())
}

// query -> cookie -> default
fn get_lang(conn: &Conn, default_lang: &str) -> String {
    let mut query_lang = None;
    let query_string = conn.querystring();
    for arg in query_string.split("&") {
        if let Some((key, value)) = arg.split_once("=") {
            if key.trim() == "lang" {
                let value = value.trim();
                query_lang = Some(value);
                break;
            }
        }
    }

    let mut cookie_lang = None;
    if let Some(cookie) = conn.request_headers().get_str("Cookie") {
        for arg in cookie.split(";") {
            if let Some((key, value)) = arg.split_once("=") {
                if key.trim() == "lang" {
                    cookie_lang = Some(value.trim());
                    break;
                }
            }
        }
    }

    query_lang.or(cookie_lang).unwrap_or(default_lang).to_owned()
}

// get configuration values from connection state
fn get_config(conn: &Conn) -> (
    Arc<TranspoConfig>, Arc<Translations>, Translation, String)
{
    let state = conn.state::<TranspoState>().unwrap().clone();
    let lang = get_lang(conn, &state.config.default_lang);
    let translation = state.translations.get(&lang);
    (state.config, state.translations, translation, lang)
}

fn set_lang_cookie(conn: &mut Conn, lang: &str) {
    conn.response_headers_mut()
        .insert("Set-Cookie", format!("lang={}; Path=.; SameSite=Lax", lang));
}

fn trillium_main(
    config: Arc<TranspoConfig>,
    translations: Arc<Translations>,
    db_pool: Arc<db::DbConnectionPool>)
{
    let accessors = Accessors::new();

    let quotas = if config.quota_bytes_total == 0 {
        None
    } else {
        Some(Quotas::from(config.as_ref()))
    };

    let storage_limit = StorageLimit::from(config.as_ref());

    spawn_cleanup_thread(
        config.read_timeout_milliseconds,
        config.storage_dir.to_owned(),
        storage_limit.clone(),
        db_pool.clone());

    let s = TranspoState {
        config: config.clone(),
        translations: translations.clone(),
        accessors: accessors.clone(),
        quotas,
        storage_limit,
        db_pool: db_pool
    };

    let router = Router::new()
        .get("/", (state(s.clone()), move |mut conn: Conn| { async move {
            let (config, translations, translation, lang) = get_config(&conn);
            set_lang_cookie(&mut conn, &lang);

            let index = IndexTemplate::new(
                &config,
                translations.names(),
                &lang,
                translation);

            conn.render(index).halt()
        }}))
        .get("/about", (state(s.clone()), move |mut conn: Conn| { async move {
            let (config, translations, translation, lang) = get_config(&conn);
            set_lang_cookie(&mut conn, &lang);
            let about = AboutTemplate::new(&config, translations.names(), &lang, translation);

            conn.render(about).halt()
        }}))
        .get("/paste", (state(s.clone()), move |mut conn: Conn| { async move {
            let (config, translations, translation, lang) = get_config(&conn);
            set_lang_cookie(&mut conn, &lang);
            let paste = PasteTemplate::new(&config, translations.names(), &lang, translation);

            conn.render(paste).halt()
        }}))
        .post("/upload", (state(s.clone()), move |mut conn: Conn| { async move {
            let (config, _, translation, _) = get_config(&conn);
            let state = conn.take_state::<TranspoState>().unwrap();
            let quota = get_quota(state.quotas, conn.request_headers());

            upload::handle_post(conn, config, quota, state.storage_limit, translation, state.db_pool).await
        }}))
        .get("/upload", (state(s.clone()), websocket(move |mut conn: WebSocketConn| { async move {
            let state = conn.take_state::<TranspoState>().unwrap();
            let quota = get_quota(state.quotas, conn.headers());

            drop(upload::handle_websocket(conn, state.config, state.db_pool, quota, state.storage_limit).await)
        }}).with_protocol_config(WS_UPLOAD_CONFIG)))
        .get("/:file_id", (state(s.clone()), move |conn: Conn| { async move {
            let file_id = conn.param("file_id").unwrap().to_owned();
            let (config, _, translation, _) = get_config(&conn);

            let mut has_password = true;
            let mut is_paste = false;
            for field in conn.querystring().split('&') {
                match field {
                    "nopass" => has_password = false,
                    "paste" => is_paste = true,
                    _ => {}
                }
            }

            if file_id.len() == ID_STRING_LENGTH {
                let conn = if is_paste {
                    conn.render(PasteDownloadTemplate {
                        app_name: &config.app_name,
                        has_password,
                        t: translation
                    })
                } else {
                    conn.render(DownloadTemplate {
                        file_id,
                        app_name: &config.app_name,
                        has_password,
                        t: translation
                    })
                };

                conn.halt()
            } else {
                http_errors::error_404(conn, config, translation)
            }
        }}))
        // TODO: decide whether or not to remove this
        /*
        .get("/:file_id/info", (state(s.clone()), move |mut conn: Conn| { async move {
            let file_id = conn.param("file_id").unwrap().to_owned();
            let (_, _, translation, _) = get_config(&conn);
            let state = conn.take_state::<TranspoState>().unwrap();

            download::info(
                conn, file_id, state.config, state.storage_limit,
                state.accessors, translation, state.db_pool).await
        }}))
        */
        .get("/:file_id/dl", (state(s.clone()), move |mut conn: Conn| { async move {
            let file_id = conn.param("file_id").unwrap().to_owned();
            let (config, _, translation, _) = get_config(&conn);
            let state = conn.take_state::<TranspoState>().unwrap();

            download::handle(
                conn, file_id, config, state.storage_limit, state.accessors, translation, state.db_pool).await
        }}))
        .get("/clear-data", move |conn: Conn| { async move {
            conn
                .with_status(200)
                .with_response_header("Clear-Site-Data", "\"storage\"")
                .with_body("Cleared site data (including service worker)")
                .halt()
        }})
        .get("/download_worker.js", files(crate_relative_path!("www/js")))
        .get("/js/*", files(crate_relative_path!("www/js")))
        .get("/css/*", files(crate_relative_path!("www/css")))
        .get("/res/*", files(crate_relative_path!("www/res")))
        .get("*", (state(s.clone()), move |mut conn: Conn| { async move {
            let (config, _, translation, _) = get_config(&mut conn);
            http_errors::error_404(conn, config, translation)
        }}));

    let rt  = Builder::new_multi_thread()
        .enable_all()
        .build().expect("Starting async runtime");
    rt.block_on(trillium_tokio::config()
        .with_host("0.0.0.0")
        .with_port(config.port as u16)
        .run_async(router));
}
