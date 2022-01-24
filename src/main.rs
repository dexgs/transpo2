mod config;
mod templates;
mod multipart_form;

use config::*;
use templates::*;

use std::env;
use trillium::Conn;
use trillium_router::Router;
use trillium_askama::AskamaConnExt;
use trillium_static::{files, crate_relative_path};

fn main() {
    let config = TranspoConfig::from(env::args());
    println!("Running with: {:?}", &config);

    let index = IndexTemplate::from(&config);

    trillium_smol::config()
        .with_host("0.0.0.0")
        .with_port(config.port as u16)
        .run(
            Router::new()
                .get("/", move |conn: Conn| async move { conn.render(index) })
                .get("/js/*", files(crate_relative_path!("www/js")))
                .get("/css/*", files(crate_relative_path!("www/css")))
                .get("/:file_id", |conn: Conn| async { conn.ok("blah!") })
                //.get("/dl/:file_id")
        );
}
