use trillium::Conn;
use trillium_askama::AskamaConnExt;
use std::sync::Arc;
use crate::config::*;
use crate::templates::*;

pub fn error_400(conn: Conn, config: Arc<TranspoConfig>) -> Conn {
    let template = ErrorTemplate {
        error_code: 400,
        message: "Bad request.".to_string(),
        app_name: config.app_name.to_owned()
    };

    conn.render(template).with_status(400).halt()
}

pub fn error_404(conn: Conn, config: Arc<TranspoConfig>) -> Conn {
    let template = ErrorTemplate {
        error_code: 404,
        message: "Page not found.".to_string(),
        app_name: config.app_name.to_owned()
    };

    conn.render(template).with_status(404).halt()
}
