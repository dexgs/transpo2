use trillium::Conn;

pub fn error_400(conn: Conn) -> Conn {
    conn.with_body("Error 400").with_status(400).halt()
}

pub fn error_404(conn: Conn) -> Conn {
    conn.with_body("Page not found").with_status(404).halt()
}
