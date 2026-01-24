use std::path::PathBuf;

use crate::settings::DownloadSettings;
use crate::util::*;
use transpo2::files::*;

use tokio::fs;
use futures_util::{TryStreamExt, io::{Error, ErrorKind}};
use urlencoding::encode;
use trillium_tokio::async_compat::CompatExt;


pub async fn main(settings: DownloadSettings) {
    let url = match settings.password {
        Some(password) => format!("{}?password={}", settings.url, encode(&password)),
        None => settings.url
    };
    let response = require(reqwest::get(url).await, "Connecting to Transpo server");

    if !response.status().is_success() {
        exit("Download failed (expired/non-existant/incorrect password)");
    }

    let ciphertext_length = response.headers()
        .get("Transpo-Ciphertext-Length")
        .map(|v| v.to_str().unwrap_or(""))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);

    let body = response.bytes_stream()
        .map_err(|e| Error::new(ErrorKind::Other, e))
        .into_async_read()
        .compat();

    let key = settings.key.into_bytes();

    let (reader, file_name, _) = require(
        EncryptedReader::with_reader(body, &key).await,
        "Receiving file name and mime type");

    let file_name = if file_name.len() > 0 {
        file_name
    } else {
        String::from("download")
    };

    let path = match settings.file_path {
        Some(file_path) => if file_path.is_dir() {
            file_path.join(file_name)
        } else {
            file_path
        },
        None => PathBuf::from(file_name)
    };

    let file = require(if settings.force {
        fs::File::create(&path).await
    } else {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path).await
    }, &format!("Opening {:?}", path));

    // Print the output path to the user
    println!("{}", path.display());

    let progress = reader.bytes_read();
    require(io_loop(reader, file, ciphertext_length, progress).await, "Downloading file");
}
