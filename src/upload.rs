use crate::concurrency::Accessors;
use crate::multipart_form::{self, *};
use trillium::Conn;
use smol::prelude::*;

pub async fn handle(mut conn: Conn, accessors: Accessors) -> Conn {
    let boundary = match get_boundary(&conn) {
        Some(boundary) => boundary,
        None => return conn.with_body("Error").with_status(500).halt()
    };
    let boundary = format!("\n--{}", boundary);
    let boundary_byte_map = byte_map(boundary.as_bytes());

    let mut req_body = conn.request_body().await;

    let mut buf = [0; 5120];
    // Make the first boundary start with a newline to simplify parsing
    (&mut buf[0..2]).copy_from_slice(&NEWLINE);
    let mut total_bytes = 0;
    let mut read_start = 2;

    while let Ok(bytes_read) = req_body.read(&mut buf[read_start..]).await {
        if bytes_read == 0 {
            break;
        } else {
            total_bytes += bytes_read;
        }

        let mut parse_start = 0;
        while buf.len() - parse_start > boundary.len() {
            let parse_result = multipart_form::parse(&buf[parse_start..], &boundary, &boundary_byte_map);
            match parse_result {
                ParseResult::NewValue(b, cd, ct, val) => {
                    parse_start += b;
                    println!("---------------------\nContent-Disposition: {}\nContent-Type: {}", cd, ct);
                },
                ParseResult::Continue(b, val) => {
                    parse_start += b;
                },
                _ => break
            }
        }

        // The buffer may contain incomplete data at the end, so we copy it to
        // the front of the buffer and make sure it doesn't get read over
        buf.copy_within(parse_start.., 0);
        read_start = buf.len() - parse_start;
    }

    println!("\nUPLOAD SIZE: {}\n", total_bytes);

    conn.ok("blah!")
}


// Read the multipart form boundary out of the headers
fn get_boundary<'a>(conn: &'a Conn) -> Option<&'a str> {
    conn.headers()
        .get_str("Content-Type")
        .and_then(|ct| ct.split_once("boundary"))
        .and_then(|(_, boundary)| boundary.split_once('='))
        .and_then(|(_, boundary)| {
            let boundary = boundary.trim();
            if boundary.starts_with('"') {
                let len = boundary.len();
                if len > 1 {
                    Some(&boundary[1..(len - 1)])
                } else {
                    None
                }
            } else {
                Some(boundary)
            }
        })
}
