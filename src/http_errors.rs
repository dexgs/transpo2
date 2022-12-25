use trillium::Conn;
use trillium_askama::AskamaConnExt;
use std::sync::Arc;
use crate::config::*;
use crate::templates::*;
use crate::translations::*;

fn path_depth(path: &str) -> usize {
    let mut depth = 0;

    for b in path.bytes() {
        if b == b'/' {
            depth += 1;
        }
    }

    depth
}

fn path_prefix(path: &str) -> String {
    "../".repeat(path_depth(path))
}

pub fn error_400(conn: Conn, config: Arc<TranspoConfig>, translation: Translation) -> Conn {
    let template = ErrorTemplate {
        error_code: 400,
        t: translation,
        app_name: &config.app_name,
        path_prefix: path_prefix(conn.path())
    };

    conn.render(template).with_status(400).halt()
}

pub fn error_404(conn: Conn, config: Arc<TranspoConfig>, translation: Translation) -> Conn {
    let template = ErrorTemplate {
        error_code: 404,
        t: translation,
        app_name: &config.app_name,
        path_prefix: path_prefix(conn.path())
    };

    conn.render(template).with_status(404).halt()
}
