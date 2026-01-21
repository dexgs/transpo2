use std::path::Path;

use futures_util::{io::copy, stream::StreamExt};
use tokio::{fs::File, io::{AsyncRead, BufReader}};
use trillium_tokio::async_compat::Compat;

use tungstenite::{Message, Bytes, client::IntoClientRequest};
use async_tungstenite::{WebSocketSender, tokio::connect_async, bytes::ByteWriter};

use urlencoding::encode;


use crate::settings::*;
use crate::util::*;
use transpo2::{upload::UploadError, files::*};


fn parse_err(bytes: Bytes) -> UploadError {
    if bytes.len() != 1 {
        exit("Unexpected message (expecting single byte error message)");
    }

    bytes[0].into()
}

fn file_name<P>(path: P) -> String
where P: AsRef<Path>
{
    let path = require(std::fs::canonicalize(path), "Canonicalizing file path");
    path.file_name().expect("Getting file name").to_string_lossy().to_string()
}

async fn upload<S, R>(
    mut writer: EncryptedWriter<ByteWriter<WebSocketSender<S>>>,
    reader: R, file_name: String, mime_type: String)
    -> Result<(), std::io::Error>
where S: futures_util::AsyncRead + futures_util::AsyncWrite + Unpin,
      R: AsyncRead
{
    writer.write_metadata(file_name, mime_type).await?;
    let mut writer = Compat::new(writer);
    let reader = Compat::new(reader);
    copy(reader, &mut writer).await?;
    let mut sender = writer.into_inner().finish().await?.into_inner();
    require(sender.close(None).await, "Closing connection");
    Ok(())
}

pub async fn main() {
    let settings = UploadSettings::from_args();

    let file = BufReader::new(require(
        File::open(&settings.file_path).await,
        &format!("Opening {:?}", &settings.file_path)));

    let minutes = format!("?minutes={}", settings.minutes);
    let password = match settings.password.as_ref() {
        Some(password) => format!("&password={}", encode(password)),
        None => String::new()
    };
    let download_limit = match settings.download_limit {
        Some(download_limit) => format!("&max-downloads={}", download_limit),
        None => String::new()
    };

    let request_url = format!("{}{}{}{}", settings.ws_url, minutes, password, download_limit);
    let request = require(request_url.into_client_request(), "Constructing client request");

    let (mut ws, _) = require(connect_async(request).await, "Connecting to Transpo server");
    let id = match require(ws.next().await.transpose(), "Receiving upload ID from Transpo server") {
        Some(Message::Text(text)) => text.as_str().to_owned(),
        Some(Message::Binary(b)) => exit(&format!("Upload error: {:?}", parse_err(b))),
        None => exit("Connection closed early without error code"),
        _ => exit("Received unexpected message (expecting upload ID)")
    };

    let (send, mut recv) = ws.split();
    let byte_writer = ByteWriter::new(send);
    let (writer, key) = EncryptedWriter::new(byte_writer).await;

    // Print upload URL to the user
    let nopass = if password.is_empty() {
        "?nopass"
    } else {
        ""
    };
    println!("{}/{}{}#{}", settings.url, id, nopass, key);

    match upload(writer, file, file_name(settings.file_path), settings.mime_type).await {
        Ok(()) => {},
        Err(e) => {
            exit(&match recv.next().await {
                Some(Ok(Message::Binary(b))) => format!("Upload failed: {:?}", parse_err(b)),
                _ => format!("Upload failed: {}", e)
            })
        }
    }
}
