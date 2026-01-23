use tokio::runtime::Builder;

mod upload;
mod download;
mod settings;
mod util;

use settings::Settings;


fn main() {
    let rt = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build().expect("Starting async runtime");
    rt.block_on(async_main())
}


async fn async_main() {
    match Settings::from_args() {
        Settings::Upload(settings) => upload::main(settings).await,
        Settings::Download(settings) => download::main(settings).await
    }
}
