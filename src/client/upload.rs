use std::path::Path;

use futures_util::stream::StreamExt;
use tokio::{fs::File, io::{AsyncReadExt, BufReader}};

use tungstenite::{
    Message, Bytes, client::IntoClientRequest, protocol::WebSocketConfig};
use async_tungstenite::{
    WebSocketSender, tokio::connect_async_with_config, bytes::ByteWriter};

use urlencoding::encode;


use crate::settings::UploadSettings;
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
    path.as_ref()
        .file_name()
        .expect("Getting file name")
        .to_string_lossy().to_string()
}

async fn upload<S, R>(
    mut writer: EncryptedWriter<ByteWriter<WebSocketSender<S>>>,
    reader: R, file_name: String, mime_type: String, total_size: usize)
    -> Result<(), std::io::Error>
where S: futures_util::AsyncRead + futures_util::AsyncWrite + Unpin,
      R: AsyncReadExt + Unpin
{
    writer.write_metadata(file_name, mime_type).await?;

    io_loop(reader, &mut writer, total_size).await?;

    let mut sender = writer.finish().await?.into_inner();
    require(sender.close(None).await, "Closing connection");

    Ok(())
}

pub async fn main(settings: UploadSettings) {
    let path = require(std::fs::canonicalize(&settings.file_path), "Canonicalizing file path");
    let file = require(
        File::open(&path).await,
        &format!("Opening {:?}", &path));
    let metadata = require(file.metadata().await, &format!("Reading metadata of {:?}", &path));
    if !metadata.is_file() {
        exit(&format!("{:?} is not a regular file", &path));
    }
    let file_size = metadata.len() as usize;
    let reader = BufReader::new(file);

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

    let ws_config = WebSocketConfig::default().max_message_size(Some(64));
    let (mut ws, _) = require(
        connect_async_with_config(request, Some(ws_config)).await,
        "Connecting to Transpo server");
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

    match upload(writer, reader, file_name(path), settings.mime_type, file_size).await {
        Ok(()) => {},
        Err(e) => {
            exit(&match recv.next().await {
                Some(Ok(Message::Binary(b))) => format!("Upload failed: {:?}", parse_err(b)),
                _ => format!("Upload failed: {}", e)
            })
        }
    }
}
